//! The gateway websocket connection, which delivers live events.
//!
//! Dispatches are read as raw JSON rather than through twilight's `Event`
//! enum: the app needs a few user-client payloads twilight either models too
//! strictly (a DM call's `VOICE_SERVER_UPDATE` has no guild id) or doesn't
//! model at all. The shard still handles the session itself — heartbeats,
//! identify, resume — since that happens as messages are polled.

mod connection;
mod dispatch;
mod payloads;
mod sender;
#[cfg(test)]
mod tests;

use std::{pin::Pin, task::Poll};

use futures::{Stream as _, StreamExt as _, channel::mpsc::UnboundedReceiver, future::poll_fn};
use serde::Deserialize;
use serde_json::value::RawValue;
use twilight_gateway::{Intents, Message as ShardMessage, Shard, ShardId};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use crate::platform::runtime;

use super::model::{
    ChannelLatest, GuildEmoji, GuildUnreads, MemberListUpdate, Message, MuteSettings,
    PresenceStatus, ReactionEmoji, ReadState, Role, StreamServerInfo, VoiceServerInfo,
    VoiceUserState,
};

pub use connection::GatewayConnectionStatus;
pub use sender::GatewaySender;

/// A message received live over the gateway, tagged with the channel it
/// belongs to so the UI can decide whether it's for the open conversation.
pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    /// `None` for a message in a DM or group DM.
    pub guild_id: Option<Id<GuildMarker>>,
    pub message: Message,
    /// The nonce the sender attached, when this is a new message. Matches the
    /// id of the optimistic copy of a message sent from here.
    pub nonce: Option<u64>,
}

pub enum GatewayEvent {
    ConnectionStatus(GatewayConnectionStatus),
    /// A new session is up. Carries the signed-in user, which voice
    /// connections need to identify themselves, and everyone in voice across
    /// its guilds — the whole of it, replacing whatever an earlier session
    /// knew, since anyone could have left while the app was disconnected.
    Ready {
        user_id: Id<UserMarker>,
        voice_states: Vec<VoiceUserState>,
    },
    Message(IncomingMessage),
    /// A message was edited. Carries the whole message as it now stands.
    MessageUpdate(IncomingMessage),
    /// One message was deleted, or several at once by a moderator.
    MessageDelete {
        channel_id: Id<ChannelMarker>,
        message_ids: Vec<Id<MessageMarker>>,
    },
    /// Someone started typing in a channel. Discord shows it for ten seconds,
    /// and clients re-send it every few while the typing goes on.
    Typing {
        channel_id: Id<ChannelMarker>,
        user_id: Id<UserMarker>,
        /// Their name in the guild. `None` in a DM, which sends no member.
        name: Option<String>,
    },
    Reaction {
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        change: ReactionChange,
    },
    /// Someone joined, left, or changed their state in a voice channel.
    VoiceState(VoiceUserState),
    /// Everyone in voice in one guild, replacing what was known of it: from
    /// `GUILD_CREATE`, or empty from `GUILD_DELETE`.
    GuildVoiceStates {
        guild_id: Id<GuildMarker>,
        states: Vec<VoiceUserState>,
    },
    /// The voice server assigned to a call the user is joining.
    VoiceServer(VoiceServerInfo),
    /// A stream was created. For the user's own, it names the server the
    /// stream connection identifies to.
    StreamCreate {
        stream_key: String,
        rtc_server_id: String,
    },
    StreamServer(StreamServerInfo),
    /// A stream ended, or the user stopped watching it. `reason` is
    /// Discord's code for why — `stream_full` and `unauthorized` when a
    /// watch was refused.
    StreamDelete {
        stream_key: String,
        reason: Option<String>,
    },
    /// Every role in a guild, from `READY` or `GUILD_CREATE`. Replaces what
    /// was known before.
    GuildRoles {
        guild_id: Id<GuildMarker>,
        roles: Vec<Role>,
    },
    RoleUpdate {
        guild_id: Id<GuildMarker>,
        role: Role,
    },
    RoleDelete {
        guild_id: Id<GuildMarker>,
        role_id: Id<RoleMarker>,
    },
    /// Every custom emoji in a guild, from `READY`, `GUILD_CREATE`, or
    /// `GUILD_EMOJIS_UPDATE`. Replaces what was known before.
    GuildEmojis {
        guild_id: Id<GuildMarker>,
        emojis: Vec<GuildEmoji>,
    },
    /// A guild member's roles, from `READY`, `GUILD_CREATE`,
    /// `GUILD_MEMBER_UPDATE`, `GUILD_MEMBERS_CHUNK`, or the member a new
    /// message carries for its author. Not only the signed-in user's: other
    /// members' roles colour their names.
    MemberRoles {
        guild_id: Id<GuildMarker>,
        user_id: Id<UserMarker>,
        roles: Vec<Id<RoleMarker>>,
    },
    /// A change to a channel member list the session is subscribed to; see
    /// [`GatewaySender::subscribe_member_list`].
    MemberList(MemberListUpdate),
    /// Other users' statuses: friends' and guild members' from `READY` and
    /// `READY_SUPPLEMENTAL`, then each change from `PRESENCE_UPDATE`.
    Presences(Vec<(Id<UserMarker>, PresenceStatus)>),
    /// The user picked a status, here or on another of their clients.
    StatusSettings(PresenceStatus),
    /// How far the user has read every channel, from `READY`. Follows
    /// `Ready`, which clears what an earlier session knew.
    ReadStates(Vec<ReadState>),
    /// The user read a channel up to a message, here or on another client.
    /// `manual` when the ack went backwards: a message marked unread.
    MessageAck {
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        manual: bool,
    },
    /// A guild's channels that count toward its unread mark, from `READY` or
    /// `GUILD_CREATE`. Replaces what was known of the guild.
    GuildUnreads(GuildUnreads),
    /// Channels' newest messages: the DMs' from `READY`, and channels that
    /// moved in guilds the session isn't sent every message for.
    LatestMessages(Vec<ChannelLatest>),
    /// What the user muted: in every guild from `READY`, in one from
    /// `USER_GUILD_SETTINGS_UPDATE`.
    MuteSettings(Vec<MuteSettings>),
}

pub enum ReactionChange {
    /// Someone reacted. Includes the signed-in user, whose own reactions also
    /// come back as dispatches.
    Add {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    Remove {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    /// Every reaction with one emoji was cleared by a moderator.
    RemoveEmoji(ReactionEmoji),
    /// Every reaction on the message was cleared by a moderator.
    RemoveAll,
}

/// The envelope every gateway payload arrives in. Only dispatches (opcode 0)
/// carry a name and data the app cares about. Both borrow from the frame, so
/// telling what a payload is costs no allocations — which matters, because
/// most of a user account's firehose is dropped unread.
#[derive(Deserialize)]
struct Envelope<'a> {
    op: u8,
    #[serde(default, borrow)]
    t: Option<&'a str>,
    #[serde(default, borrow)]
    d: Option<&'a RawValue>,
}

pub async fn receive_gateway_events(
    mut events: UnboundedReceiver<GatewayEvent>,
    mut on_event: impl FnMut(GatewayEvent) -> bool,
) {
    while let Some(event) = events.next().await {
        if !on_event(event) {
            return;
        }
    }
    // Incluye el cierre inesperado de la tarea productora, no solo el del shard.
    on_event(GatewayEvent::ConnectionStatus(
        GatewayConnectionStatus::Disconnected,
    ));
}

/// Abre el websocket y entrega cambios de conexión y los dispatches utilizados.
/// Si `on_event` devuelve `false`, termina el shard y libera el socket.
///
/// Twilight conserva la responsabilidad de reconectar, reanudar y enviar heartbeats.
pub fn connect_gateway(
    token: String,
    mut on_event: impl FnMut(GatewayEvent) -> bool + Send + 'static,
) -> GatewaySender {
    // The shard is built here rather than inside the task because the sender
    // has to come back to the caller — but building one starts the identify
    // queue's timer, so it has to happen inside the runtime all the same.
    let guard = runtime::handle().enter();
    // Discord ignores the intents field for user tokens; a real user client
    // receives every event its account can see. Request all intents so we
    // mirror that and never filter events at this layer (the shard still
    // requires a value in the IDENTIFY payload).
    let mut shard = Shard::new(ShardId::ONE, token, Intents::all());
    let sender = GatewaySender {
        inner: shard.sender(),
    };
    drop(guard);

    runtime::handle().spawn(async move {
        let mut self_id = None;
        let mut status = GatewayConnectionStatus::Connecting;
        loop {
            let item = poll_fn(|cx| {
                let poll = Pin::new(&mut shard).poll_next(cx);
                // Twilight puede cerrar el transporte y empezar a reconectar
                // dentro de un mismo poll sin entregar un frame al consumidor.
                if let Some(status) = status.observe_poll(shard.state(), &poll)
                    && !on_event(GatewayEvent::ConnectionStatus(status))
                {
                    return Poll::Ready(None);
                }
                poll
            })
            .await;
            let Some(item) = item else {
                break;
            };
            let Ok(ShardMessage::Text(json)) = item else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Envelope>(&json) else {
                continue;
            };
            if let Some(status) = status.confirm_session(shard.state(), &envelope)
                && !on_event(GatewayEvent::ConnectionStatus(status))
            {
                return;
            }
            let Envelope {
                op: 0,
                t: Some(name),
                d: Some(data),
            } = envelope
            else {
                continue;
            };

            // A dispatch the app doesn't handle, or one whose shape doesn't
            // match, yields nothing rather than ending the loop.
            for event in dispatch::dispatch(name, data, &mut self_id) {
                if !on_event(event) {
                    return;
                }
            }
        }
    });

    sender
}
