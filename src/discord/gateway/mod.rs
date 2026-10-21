//! The gateway websocket connection, which delivers live events.
//!
//! Dispatches are read as raw JSON rather than through twilight's `Event`
//! enum: the app needs a few user-client payloads twilight either models too
//! strictly (a DM call's `VOICE_SERVER_UPDATE` has no guild id) or doesn't
//! model at all. The shard still handles the session itself — heartbeats,
//! identify, resume — since that happens as messages are polled.

mod dispatch;
mod payloads;
mod sender;
#[cfg(test)]
mod tests;

use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::value::RawValue;
use twilight_gateway::{Intents, Message as ShardMessage, Shard, ShardId};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use crate::platform::runtime;

use super::model::{
    Channel, ChannelLatest, GuildEmoji, GuildMember, GuildUnreads, MemberListUpdate, Message,
    MuteSettings, PresenceStatus, ReactionEmoji, ReadState, Role, StreamServerInfo,
    VoiceServerInfo, VoiceUserState,
};

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
    /// A DM's call has started, or was asked after: who's in it, and who it's
    /// ringing. Replaces what was known of that call.
    CallCreate {
        channel_id: Id<ChannelMarker>,
        ringing: Vec<Id<UserMarker>>,
        voice_states: Vec<VoiceUserState>,
    },
    /// Who a DM's call is ringing changed.
    CallUpdate {
        channel_id: Id<ChannelMarker>,
        ringing: Vec<Id<UserMarker>>,
    },
    /// A DM's call ended: everyone has left it.
    CallDelete {
        channel_id: Id<ChannelMarker>,
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
    /// Members named by a `GUILD_MEMBERS_CHUNK`: the answer to a search from
    /// [`GatewaySender::search_members`], or to a request for messages'
    /// authors.
    Members {
        guild_id: Id<GuildMarker>,
        members: Vec<GuildMember>,
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
    /// Channels holding unread mentions, with how many, from `READY`'s read
    /// states. Follows [`GatewayEvent::ReadStates`], which reports the same
    /// channels' acks; together they replace what an earlier session knew.
    MentionCounts(Vec<(Id<ChannelMarker>, u32)>),
    /// A guild's channels that count toward its unread mark, from `READY` or
    /// `GUILD_CREATE`. Replaces what was known of the guild.
    GuildUnreads(GuildUnreads),
    /// Every channel the user can see in a guild, from `READY` or
    /// `GUILD_CREATE`: the sidebar's whole list, so opening the guild needs
    /// no request. Replaces what was known of the guild.
    GuildChannels {
        guild_id: Id<GuildMarker>,
        channels: Vec<Channel>,
    },
    /// A channel in a guild was created, changed, or deleted, so the list
    /// `GuildChannels` gave is out of date.
    GuildChannelsChanged {
        guild_id: Id<GuildMarker>,
    },
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
    #[serde(default, borrow)]
    t: Option<&'a str>,
    #[serde(default, borrow)]
    d: Option<&'a RawValue>,
}

/// Opens a gateway websocket connection and invokes `on_event` for every
/// dispatch the app acts on. Returning `false` from it (the receiving end went
/// away) ends the shard loop and drops the socket.
///
/// The shard reconnects and resumes on its own, so transient errors are
/// skipped rather than treated as fatal.
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
        while let Some(item) = shard.next().await {
            // Reconnects and resumes are handled by the shard internally; a
            // receive error just means skip this one and keep listening.
            let Ok(ShardMessage::Text(json)) = item else {
                continue;
            };
            let Ok(Envelope {
                t: Some(name),
                d: Some(data),
            }) = serde_json::from_str::<Envelope>(&json)
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
