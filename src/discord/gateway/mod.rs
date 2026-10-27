//! The gateway websocket connection, which delivers live events.
//!
//! Dispatches are read as raw JSON, not through twilight's `Event`: the app needs
//! user-client payloads twilight models too strictly (a DM call's `VOICE_SERVER_UPDATE`
//! has no guild id) or not at all. The shard still handles heartbeats, identify and resume.

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
    MuteSettings, PresenceStatus, ReactionEmoji, ReadState, Role, StreamServerInfo, UnreadSetting,
    VoiceServerInfo, VoiceUserState,
};

pub use sender::GatewaySender;

pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    /// `None` for a message in a DM or group DM.
    pub guild_id: Option<Id<GuildMarker>>,
    pub message: Message,
    /// The sender's nonce; matches the id of the optimistic copy of a message sent from here.
    pub nonce: Option<u64>,
}

pub enum GatewayEvent {
    /// Replaces everything an earlier session knew about voice, since anyone could have
    /// left while disconnected. Carries the signed-in user, which voice connections identify with.
    Ready {
        user_id: Id<UserMarker>,
        voice_states: Vec<VoiceUserState>,
    },
    Message(IncomingMessage),
    MessageUpdate(IncomingMessage),
    MessageDelete {
        channel_id: Id<ChannelMarker>,
        message_ids: Vec<Id<MessageMarker>>,
    },
    /// Shown for ten seconds; clients re-send every few while typing goes on.
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
    VoiceState(VoiceUserState),
    /// Replaces what was known of the guild; empty from `GUILD_DELETE`.
    GuildVoiceStates {
        guild_id: Id<GuildMarker>,
        states: Vec<VoiceUserState>,
    },
    /// Replaces what was known of that call.
    CallCreate {
        channel_id: Id<ChannelMarker>,
        ringing: Vec<Id<UserMarker>>,
        voice_states: Vec<VoiceUserState>,
    },
    CallUpdate {
        channel_id: Id<ChannelMarker>,
        ringing: Vec<Id<UserMarker>>,
    },
    CallDelete {
        channel_id: Id<ChannelMarker>,
    },
    VoiceServer(VoiceServerInfo),
    /// For the user's own, names the server the stream connection identifies to.
    StreamCreate {
        stream_key: String,
        rtc_server_id: String,
    },
    StreamServer(StreamServerInfo),
    /// `reason` is Discord's code, e.g. `stream_full` or `unauthorized` when a watch was refused.
    StreamDelete {
        stream_key: String,
        reason: Option<String>,
    },
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
    GuildUploadLimit {
        guild_id: Id<GuildMarker>,
        limit: u64,
    },
    GuildDefaultUnreads {
        guild_id: Id<GuildMarker>,
        unreads: UnreadSetting,
    },
    GuildEmojis {
        guild_id: Id<GuildMarker>,
        emojis: Vec<GuildEmoji>,
    },
    /// Not only the signed-in user's: other members' roles colour their names.
    MemberRoles {
        guild_id: Id<GuildMarker>,
        user_id: Id<UserMarker>,
        roles: Vec<Id<RoleMarker>>,
    },
    /// Answers [`GatewaySender::search_members`] or a request for messages' authors.
    Members {
        guild_id: Id<GuildMarker>,
        members: Vec<GuildMember>,
    },
    MemberList(MemberListUpdate),
    Presences(Vec<(Id<UserMarker>, PresenceStatus)>),
    StatusSettings(PresenceStatus),
    /// Follows `Ready`, which clears what an earlier session knew.
    ReadStates(Vec<ReadState>),
    /// The user read a channel up to a message, here or on another client.
    /// `manual` when the ack went backwards: a message marked unread.
    MessageAck {
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        manual: bool,
        /// `None` leaves the mention count as it was.
        mention_count: Option<u32>,
    },
    GuildUnreads(GuildUnreads),
    /// Replaces what was known of the guild, so opening it needs no request.
    GuildChannels {
        guild_id: Id<GuildMarker>,
        channels: Vec<Channel>,
    },
    GuildChannelsChanged {
        guild_id: Id<GuildMarker>,
    },
    LatestMessages(Vec<ChannelLatest>),
    MuteSettings(Vec<MuteSettings>),
}

pub enum ReactionChange {
    /// Includes the signed-in user, whose own reactions also come back as dispatches.
    Add {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    Remove {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    RemoveEmoji(ReactionEmoji),
    RemoveAll,
}

/// The envelope every payload arrives in. Both fields borrow from the frame so
/// classifying costs no allocations; most of a user account's firehose is dropped unread.
#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(default, borrow)]
    t: Option<&'a str>,
    #[serde(default, borrow)]
    d: Option<&'a RawValue>,
}

/// Opens the gateway and calls `on_event` for each dispatch the app acts on.
/// Returning `false` (receiver gone) ends the loop and drops the socket.
pub fn connect_gateway(
    token: String,
    mut on_event: impl FnMut(GatewayEvent) -> bool + Send + 'static,
) -> GatewaySender {
    // Built here so the sender can go back to the caller, but building starts the
    // identify queue's timer, so it must happen inside the runtime.
    let guard = runtime::handle().enter();
    // Discord ignores intents for user tokens and sends everything the account can see;
    // IDENTIFY still requires a value, so request all.
    let mut shard = Shard::new(ShardId::ONE, token, Intents::all());
    let sender = GatewaySender {
        inner: shard.sender(),
    };
    drop(guard);

    runtime::handle().spawn(async move {
        let mut self_id = None;
        while let Some(item) = shard.next().await {
            // The shard reconnects and resumes itself; just skip receive errors.
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

            for event in dispatch::dispatch(name, data, &mut self_id) {
                if !on_event(event) {
                    return;
                }
            }
        }
    });

    sender
}
