//! The gateway connection that keeps the open conversation live, and the call
//! events that ride alongside it.

use gpui::*;
use twilight_model::id::Id;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::voice::stream::StreamEvent;
use crate::voice::{VoiceEngine, VoiceEvent};

impl HomeScreen {
    /// Opens the gateway connection and pumps its events onto the gpui
    /// foreground, where they update the open conversation and the call.
    ///
    /// The call engine is started here too: a call is opened with a gateway
    /// command and answered with gateway dispatches, so neither half is of any
    /// use without the other.
    pub(in crate::screens::home) fn start_gateway(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::mpsc::unbounded::<discord::GatewayEvent>();
        let gateway = discord::connect_gateway(token, move |event| {
            // Returns whether the foreground receiver is still around; once it
            // isn't (the screen was dropped), the gateway loop stops.
            tx.unbounded_send(event).is_ok()
        });
        self.gateway = Some(gateway);

        // In the window, since a channel list arriving for the open guild
        // opens one of its channels.
        cx.spawn_in(window, async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_gateway_event(event, window, cx)
                    })
                    .is_err()
                {
                    // The entity is gone; stop draining so the sender closes.
                    break;
                }
            }
        })
        .detach();

        let (tx, rx) = futures::channel::mpsc::unbounded::<VoiceEvent>();
        let engine = VoiceEngine::start(tx);
        // Hand over the remembered microphone before any call can be made.
        engine.set_input_device(self.voice_input_device.clone());
        self.voice_engine = Some(engine);

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_voice_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (tx, rx) = futures::channel::mpsc::unbounded::<StreamEvent>();
        self.stream_events = Some(tx);

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(event) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_stream_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_gateway_event(
        &mut self,
        event: discord::GatewayEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            discord::GatewayEvent::Ready {
                user_id,
                voice_states,
            } => {
                self.self_user_id = Some(user_id);
                // A new session starts out online whatever was picked, and
                // a resume doesn't come through here, so this covers both
                // the first connect and every reconnect.
                if let (Some(status), Some(gateway)) = (self.presence_status, &self.gateway) {
                    gateway.update_presence(status);
                }
                self.replace_voice_states(None, voice_states, cx);
                // The read states follow, whole, and anything kept from an
                // earlier session could be stale.
                self.unreads = Default::default();
                // Likewise the presences, and anyone who went offline in the
                // meantime won't be among them.
                self.user_presences.clear();
                // A member asked for in a session that dropped before
                // answering is worth asking for again.
                self.requested_members.clear();
                // Subscriptions don't outlive the session.
                self.resubscribe_member_list();
                // Every guild's channels follow, but any the session sends
                // as unavailable won't be among them, and channel changes
                // made while disconnected were never heard of.
                self.stale_guild_channels
                    .extend(self.guild_channels.keys().copied());
                cx.notify();
            }
            discord::GatewayEvent::Presences(presences) => {
                // Most of these are guild members nobody here is looking at,
                // so only a change to someone in the DM list redraws.
                let mut changed = false;
                for (user_id, status) in presences {
                    if self.user_presences.insert(user_id, status) != Some(status) {
                        changed |= self.dms.iter().any(|dm| dm.recipient_id == Some(user_id));
                    }
                }
                if changed {
                    cx.notify();
                }
            }
            discord::GatewayEvent::StatusSettings(status) => {
                self.presence_status = Some(status);
                cx.notify();
            }
            discord::GatewayEvent::Message(incoming) => {
                self.unreads
                    .note_message(incoming.channel_id, incoming.message.id);
                // Discord acks a sender's own message for them.
                if Some(incoming.message.author_id) == self.self_user_id {
                    self.unreads.ack(incoming.channel_id, incoming.message.id);
                }
                if incoming.guild_id.is_none() {
                    self.note_dm_activity(incoming.channel_id, incoming.message.id, cx);
                }
                self.mentions_on_message(&incoming, cx);
                self.stop_typing(incoming.channel_id, incoming.message.author_id, cx);
                self.history_cache
                    .push(incoming.channel_id, incoming.message.clone());
                self.handle_incoming_message(incoming, cx)
            }
            discord::GatewayEvent::MessageUpdate(incoming) => {
                self.history_cache
                    .edit(incoming.channel_id, incoming.message.clone());
                self.handle_message_update(incoming, cx)
            }
            discord::GatewayEvent::MessageDelete {
                channel_id,
                message_ids,
            } => {
                self.history_cache.delete(channel_id, &message_ids);
                self.handle_message_delete(channel_id, &message_ids, cx)
            }
            discord::GatewayEvent::Typing {
                channel_id,
                user_id,
                name,
            } => self.handle_typing(channel_id, user_id, name, cx),
            discord::GatewayEvent::Reaction {
                channel_id,
                message_id,
                change,
            } => self.handle_reaction(channel_id, message_id, change, cx),
            discord::GatewayEvent::VoiceState(state) => self.handle_voice_state(state, cx),
            discord::GatewayEvent::GuildVoiceStates { guild_id, states } => {
                self.replace_voice_states(Some(guild_id), states, cx)
            }
            discord::GatewayEvent::CallCreate {
                channel_id,
                ringing,
                voice_states,
            } => self.handle_call_create(channel_id, ringing, voice_states, cx),
            discord::GatewayEvent::CallUpdate {
                channel_id,
                ringing,
            } => {
                self.call_ringing.insert(channel_id, ringing);
                cx.notify();
            }
            discord::GatewayEvent::CallDelete { channel_id } => {
                self.handle_call_delete(channel_id, cx)
            }
            discord::GatewayEvent::VoiceServer(server) => self.handle_voice_server(server, cx),
            discord::GatewayEvent::StreamCreate {
                stream_key,
                rtc_server_id,
            } => self.handle_stream_create(stream_key, rtc_server_id, cx),
            discord::GatewayEvent::StreamServer(server) => self.handle_stream_server(server, cx),
            discord::GatewayEvent::StreamDelete { stream_key, reason } => {
                self.handle_stream_delete(stream_key, reason, cx)
            }
            discord::GatewayEvent::GuildRoles { guild_id, roles } => {
                let roles = roles.into_iter().map(|role| (role.id, role)).collect();
                self.guild_roles.insert(guild_id, roles);
                cx.notify();
            }
            discord::GatewayEvent::RoleUpdate { guild_id, role } => {
                // A role's permissions decide which channels its members see.
                self.invalidate_guild_channels(guild_id, window, cx);
                self.guild_roles
                    .entry(guild_id)
                    .or_default()
                    .insert(role.id, role);
                cx.notify();
            }
            discord::GatewayEvent::RoleDelete { guild_id, role_id } => {
                self.invalidate_guild_channels(guild_id, window, cx);
                if let Some(roles) = self.guild_roles.get_mut(&guild_id) {
                    roles.remove(&role_id);
                }
                cx.notify();
            }
            discord::GatewayEvent::GuildEmojis { guild_id, emojis } => {
                self.guild_emojis.insert(guild_id, emojis);
            }
            discord::GatewayEvent::ReadStates(states) => {
                self.unreads.set_read_states(states);
                // Acks made before the session came up are newer than what
                // it reports, and one sent before it connected never echoes
                // back — so keep the unsent ones and read the open
                // conversation again.
                for (&channel_id, &message_id) in &self.pending_acks {
                    self.unreads.ack(channel_id, message_id);
                }
                self.ack_open_conversation(cx);
                cx.notify();
            }
            discord::GatewayEvent::MessageAck {
                channel_id,
                message_id,
                manual,
            } => {
                if manual {
                    self.unreads.set_acked(channel_id, message_id);
                } else {
                    self.unreads.ack(channel_id, message_id);
                    // Reading a channel here or on another client takes its
                    // mentions with it. A manual one went backwards — mark
                    // unread — and what its mentions were isn't known.
                    self.mentions_clear(channel_id, cx);
                }
                cx.notify();
            }
            discord::GatewayEvent::GuildUnreads(guild) => {
                self.mentions_on_guild_unreads(&guild);
                self.unreads.set_guild(guild);
                cx.notify();
            }
            discord::GatewayEvent::MentionCounts(counts) => self.mentions_on_ready(counts, cx),
            discord::GatewayEvent::GuildChannels { guild_id, channels } => {
                self.set_guild_channels(guild_id, channels, window, cx);
            }
            discord::GatewayEvent::GuildChannelsChanged { guild_id } => {
                self.invalidate_guild_channels(guild_id, window, cx);
            }
            discord::GatewayEvent::LatestMessages(channels) => {
                self.unreads.set_latest(channels);
                cx.notify();
            }
            discord::GatewayEvent::MuteSettings(settings) => {
                self.unreads.set_mutes(settings);
                cx.notify();
            }
            discord::GatewayEvent::MemberList(update) => {
                self.apply_member_list_update(update, cx);
            }
            discord::GatewayEvent::Members { guild_id, members } => {
                self.add_guild_members(guild_id, members, cx);
            }
            discord::GatewayEvent::MemberRoles {
                guild_id,
                user_id,
                roles,
            } => {
                let previous = self
                    .member_roles
                    .entry(guild_id)
                    .or_default()
                    .insert(user_id, roles.clone());
                // The user's own roles decide which channels they see. Every
                // message they send carries them too, unchanged.
                if Some(user_id) == self.self_user_id
                    && previous.is_some_and(|previous| previous != roles)
                {
                    self.invalidate_guild_channels(guild_id, window, cx);
                }
                cx.notify();
            }
        }
    }

    /// Appends a live message to the open conversation, if it belongs there.
    fn handle_incoming_message(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(incoming.channel_id) {
            return;
        }
        // The history for this channel is still loading and will replace the
        // whole list when it lands (and include this message), so skip it now
        // to avoid a desync between `messages` and the list state.
        if self.messages_loading {
            return;
        }
        // Open further back than the present, the list stops short of the
        // newest messages, and this one would sit after the gap. Paging
        // forward brings it in.
        if !self.reached_newest {
            return;
        }
        // The echo of a message sent from here takes the place of its pending
        // copy.
        if let Some(nonce) = incoming.nonce.and_then(Id::new_checked)
            && self.confirm_pending_message(nonce, incoming.message.clone())
        {
            cx.notify();
            return;
        }
        // Otherwise it's a duplicate: a message sent from here whose response
        // got in first, or a repeated dispatch.
        if let Some(existing) = self
            .messages
            .iter_mut()
            .find(|message| message.id == incoming.message.id)
        {
            // The send's response has no guild member on it, so the echo is
            // what carries the author's nickname.
            existing.author_name = incoming.message.author_name;
            cx.notify();
            return;
        }

        // Arriving while the user is up reading history, it's unread: the
        // divider goes above it, unless one is already further back.
        if !self.at_bottom && self.unread_marker.is_none() {
            self.unread_marker = self
                .unreads
                .acked(incoming.channel_id)
                .filter(|&acked| acked < incoming.message.id);
        }
        let ix = self.messages.len();
        self.messages.push(incoming.message);
        self.splice_messages(ix..ix, 1);
        // Follow the conversation only when the newest message was already in
        // view; if the user has scrolled up to read history, leave them there.
        if self.at_bottom {
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
            self.ack_open_conversation(cx);
        }
        cx.notify();
    }
}
