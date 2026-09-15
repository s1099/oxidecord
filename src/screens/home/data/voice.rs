//! Joining, leaving, and listening to a call.
//!
//! Joining a voice channel is a gateway command, not a request: Discord
//! answers it with a `VOICE_STATE_UPDATE` carrying the session and a
//! `VOICE_SERVER_UPDATE` carrying the server to talk to. Only once both have
//! arrived can [`crate::voice`] open the connection, so a join is held in
//! [`PendingVoice`] until then.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord;
use crate::platform::prefs;
use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{
    PendingVoice, VoiceCall, VoiceKind, VoiceParticipant, VoiceStatus,
};
use crate::voice::{VoiceConnection, VoiceEvent};

impl HomeScreen {
    /// Whether the active call is in this channel.
    pub(in crate::screens::home) fn in_voice_channel(&self, channel_id: Id<ChannelMarker>) -> bool {
        self.voice
            .as_ref()
            .is_some_and(|call| call.channel_id == channel_id)
    }

    /// Joins a guild voice channel, or leaves it if it's the one already
    /// joined — clicking the connected channel again toggles, like Discord.
    pub(in crate::screens::home) fn join_voice_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        cx: &mut Context<Self>,
    ) {
        if self.in_voice_channel(channel_id) {
            self.leave_voice(cx);
            return;
        }

        let Some(channel) = self
            .channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == channel_id)
        else {
            return;
        };
        let name = channel.name.clone();
        let guild_id = self.selected_guild;
        let guild_name = guild_id
            .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
            .map(|guild| guild.name.clone());

        self.open_call(
            VoiceCall {
                channel_id,
                guild_id,
                kind: VoiceKind::Channel,
                name,
                context: guild_name,
                status: VoiceStatus::Connecting,
                error: None,
            },
            cx,
        );
        // Opening the channel puts its stage in the content pane.
        self.selected_channel = Some(channel_id);
    }

    /// Places a call in the open DM conversation. Discord treats a DM call as
    /// a guildless voice channel, so it joins the same way.
    pub(in crate::screens::home) fn start_dm_call(&mut self, cx: &mut Context<Self>) {
        let Some(dm) = self.selected_dm_info().cloned() else {
            return;
        };
        if self.in_voice_channel(dm.id) {
            return;
        }

        self.open_call(
            VoiceCall {
                channel_id: dm.id,
                guild_id: None,
                kind: VoiceKind::Direct,
                name: dm.name,
                context: None,
                status: VoiceStatus::Connecting,
                error: None,
            },
            cx,
        );
    }

    /// Announces the join on the gateway and waits for the voice server.
    fn open_call(&mut self, call: VoiceCall, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            return;
        };

        gateway.update_voice_state(
            call.guild_id,
            Some(call.channel_id),
            self.voice_muted,
            self.voice_deafened,
        );
        self.voice = Some(call);
        self.pending_voice = Some(PendingVoice::default());
        self.voice_speaking.clear();
        cx.notify();
    }

    pub(in crate::screens::home) fn leave_voice(&mut self, cx: &mut Context<Self>) {
        let guild_id = self.voice.as_ref().and_then(|call| call.guild_id);
        if let Some(gateway) = &self.gateway {
            gateway.update_voice_state(guild_id, None, self.voice_muted, self.voice_deafened);
        }
        if let Some(engine) = &self.voice_engine {
            engine.disconnect();
        }
        self.voice = None;
        self.pending_voice = None;
        self.voice_speaking.clear();
        cx.notify();
    }

    /// Toggles self-mute: whether the microphone is sent.
    ///
    /// Unmuting while deafened undeafens as well, since deafening is what
    /// muted the microphone in the first place.
    pub(in crate::screens::home) fn toggle_voice_mute(&mut self, cx: &mut Context<Self>) {
        self.voice_muted = !self.voice_muted;
        if !self.voice_muted {
            self.voice_deafened = false;
        }
        self.voice_mute_before_deafen = self.voice_muted;
        self.apply_voice_flags(cx);
    }

    /// Toggles self-deafen: whether everyone else is played. Deafening stops
    /// the microphone too, and undeafening hands back whatever mute state the
    /// user had before.
    pub(in crate::screens::home) fn toggle_voice_deafen(&mut self, cx: &mut Context<Self>) {
        self.voice_deafened = !self.voice_deafened;
        if self.voice_deafened {
            self.voice_mute_before_deafen = self.voice_muted;
            self.voice_muted = true;
        } else {
            self.voice_muted = self.voice_mute_before_deafen;
        }
        self.apply_voice_flags(cx);
    }

    /// Picks the microphone to use, for this call and every later one.
    ///
    /// `None` means whichever device the system calls the default, which is
    /// also where an unplugged choice falls back to.
    pub(in crate::screens::home) fn set_input_device(
        &mut self,
        device: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.voice_input_device = device.clone();
        prefs::update(|prefs| prefs.input_device = device.clone());
        if let Some(engine) = &self.voice_engine {
            engine.set_input_device(device);
        }
        cx.notify();
    }

    /// Pushes mute and deafen to the call and to the gateway. The gateway half
    /// is what everyone else sees on the user's tile.
    fn apply_voice_flags(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = &self.voice_engine {
            engine.set_muted(self.voice_muted);
            engine.set_deafened(self.voice_deafened);
        }
        if let Some(call) = &self.voice
            && let Some(gateway) = &self.gateway
        {
            gateway.update_voice_state(
                call.guild_id,
                Some(call.channel_id),
                self.voice_muted,
                self.voice_deafened,
            );
        }
        cx.notify();
    }

    /// Records where someone is in voice. The user's own state also carries
    /// the session id a connection needs, and says when they've been moved or
    /// disconnected from the far end.
    pub(in crate::screens::home) fn handle_voice_state(
        &mut self,
        state: discord::VoiceUserState,
        cx: &mut Context<Self>,
    ) {
        let user_id = state.user_id;
        let is_self = Some(user_id) == self.self_user_id;
        let channel_id = state.channel_id;

        // Gateway dispatches don't always repeat the member, so a state that
        // arrives without one keeps the name and avatar already known.
        let known = self.voice_states.get(&user_id);
        let state = discord::VoiceUserState {
            name: state
                .name
                .or_else(|| known.and_then(|state| state.name.clone())),
            avatar_url: state
                .avatar_url
                .or_else(|| known.and_then(|state| state.avatar_url.clone())),
            ..state
        };

        if channel_id.is_some() {
            self.voice_states.insert(user_id, state.clone());
        } else {
            // No channel means they left voice entirely.
            self.voice_states.remove(&user_id);
        }

        if is_self && self.voice.is_some() {
            match channel_id {
                Some(channel_id) => {
                    // A moderator can move the user between channels. Follow
                    // them: the voice server for the new channel is on its way.
                    if self
                        .voice
                        .as_ref()
                        .is_some_and(|call| call.channel_id != channel_id)
                    {
                        self.follow_move(channel_id, state.guild_id);
                    }
                    // The session id completes half of what a connection needs.
                    let pending = self.pending_voice.get_or_insert_with(PendingVoice::default);
                    pending.session_id = Some(state.session_id.clone());
                    self.try_connect_voice(cx);

                    // The far end can mute or deafen the user; mirror it so the
                    // buttons match what everyone else sees.
                    self.voice_muted = state.self_mute;
                    self.voice_deafened = state.self_deaf;
                }
                // Disconnected from the far end: the call is already over, so
                // only the local side is torn down.
                None => {
                    if let Some(engine) = &self.voice_engine {
                        engine.disconnect();
                    }
                    self.voice = None;
                    self.pending_voice = None;
                    self.voice_speaking.clear();
                }
            }
        }

        cx.notify();
    }

    /// Points the open call at a channel the user was moved to, and starts
    /// waiting for that channel's voice server.
    fn follow_move(&mut self, channel_id: Id<ChannelMarker>, guild_id: Option<Id<GuildMarker>>) {
        // A move within the guild is the common case; a name the app hasn't
        // loaded keeps the one already on the call rather than blanking it.
        let name = self
            .channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == channel_id)
            .map(|channel| channel.name.clone());

        let mut previous = None;
        if let Some(call) = &mut self.voice {
            previous = Some(call.channel_id);
            call.channel_id = channel_id;
            call.guild_id = guild_id;
            call.status = VoiceStatus::Connecting;
            call.error = None;
            if let Some(name) = name {
                call.name = name;
            }
        }
        self.pending_voice = Some(PendingVoice::default());
        self.voice_speaking.clear();
        // Follow the move on screen only if the old channel was the one open;
        // someone reading a text channel stays where they are.
        if self.selected_channel == previous {
            self.selected_channel = Some(channel_id);
        }
    }

    /// Records the voice server for the join in flight.
    pub(in crate::screens::home) fn handle_voice_server(
        &mut self,
        server: discord::VoiceServerInfo,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = &self.voice else {
            return;
        };
        // Updates for another guild's call aren't this one's business. A DM
        // call carries no guild, and its channel id instead.
        let matches = match (server.guild_id, server.channel_id) {
            (Some(guild_id), _) => call.guild_id == Some(guild_id),
            (None, Some(channel_id)) => call.channel_id == channel_id,
            (None, None) => true,
        };
        if !matches {
            return;
        }

        let pending = self.pending_voice.get_or_insert_with(PendingVoice::default);
        pending.token = Some(server.token);
        // A server update with no endpoint means Discord is reallocating the
        // call; the next one carries the new server.
        if let Some(endpoint) = server.endpoint {
            pending.endpoint = Some(endpoint);
        }
        self.try_connect_voice(cx);
    }

    /// Opens the connection once the gateway has delivered every part of it.
    fn try_connect_voice(&mut self, cx: &mut Context<Self>) {
        let (Some(call), Some(pending), Some(user_id)) =
            (&self.voice, &self.pending_voice, self.self_user_id)
        else {
            return;
        };
        let (Some(session_id), Some(token), Some(endpoint)) = (
            pending.session_id.clone(),
            pending.token.clone(),
            pending.endpoint.clone(),
        ) else {
            return;
        };

        let connection = VoiceConnection {
            channel_id: call.channel_id,
            guild_id: call.guild_id,
            user_id,
            session_id,
            token,
            endpoint,
        };
        // Taken rather than kept: a later reconnect arrives as a fresh pair of
        // dispatches, and reusing a stale token would be rejected.
        self.pending_voice = None;

        let Some(engine) = &self.voice_engine else {
            return;
        };
        engine.set_muted(self.voice_muted);
        engine.set_deafened(self.voice_deafened);
        engine.connect(connection);
        cx.notify();
    }

    /// Applies what the call reports back.
    pub(in crate::screens::home) fn handle_voice_event(
        &mut self,
        event: VoiceEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            VoiceEvent::Connected => {
                if let Some(call) = &mut self.voice {
                    call.status = VoiceStatus::Connected;
                    call.error = None;
                }
            }
            VoiceEvent::Disconnected => {
                // The connection dropped on its own; tell the gateway so the
                // user doesn't linger in the channel for everyone else.
                if self.voice.is_some() {
                    self.leave_voice(cx);
                }
            }
            VoiceEvent::Speaking(speaking) => {
                self.voice_speaking = speaking;
            }
            VoiceEvent::Failed(error) => {
                if let Some(call) = &mut self.voice {
                    call.error = Some(error);
                }
            }
        }
        cx.notify();
    }

    /// Everyone in a voice channel, in a stable order: the signed-in user
    /// first, then the rest by name.
    pub(in crate::screens::home) fn voice_participants(
        &self,
        channel_id: Id<ChannelMarker>,
    ) -> Vec<VoiceParticipant> {
        let mut participants: Vec<_> = self
            .voice_states
            .values()
            .filter(|state| state.channel_id == Some(channel_id))
            .map(|state| {
                let is_self = Some(state.user_id) == self.self_user_id;
                // The user's own name and avatar are already loaded, and a DM
                // call's voice states carry no member to read them from.
                let current = self.current_user.as_ref().filter(|_| is_self);

                VoiceParticipant {
                    user_id: state.user_id,
                    name: current
                        .map(|user| user.name.clone())
                        .or_else(|| state.name.clone())
                        .unwrap_or_else(|| "Unknown".into()),
                    avatar_url: current
                        .and_then(|user| user.avatar_url.clone())
                        .or_else(|| state.avatar_url.clone()),
                    muted: state.self_mute || state.mute,
                    deafened: state.self_deaf || state.deaf,
                    speaking: self.voice_speaking.contains(&state.user_id),
                    is_self,
                }
            })
            .collect();

        participants.sort_by(|a, b| {
            b.is_self
                .cmp(&a.is_self)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        participants
    }
}
