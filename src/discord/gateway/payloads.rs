//! The shapes dispatches arrive in, of which only what the app acts on is
//! read.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};
use twilight_model::util::Timestamp;

use crate::discord::model::{
    PresenceStatus, RawChannelUnread, RawEmoji, RawGuildChannel, RawGuildSettings, RawMember,
    RawReadState, RawRole, RawVoiceState, versioned_list,
};

/// One entry of `READY`'s `read_state`, of which only the unread mention
/// count is read. The same list is parsed for acks by the model's
/// `RawReadState`, which keeps nothing else, so the two read it apart rather
/// than the model growing a field only the badges use.
#[derive(Deserialize)]
pub(super) struct RawMentionState {
    id: Id<ChannelMarker>,
    #[serde(default)]
    read_state_type: u8,
    /// Docs mark it optional and only meaningful for channel read states,
    /// which a negative count would break — treated as none, like an absent
    /// one.
    #[serde(default)]
    mention_count: i64,
}

/// The `read_state` list of `READY`, plain or versioned like the model's.
#[derive(Deserialize)]
pub(super) struct MentionStatesPayload {
    #[serde(default, deserialize_with = "versioned_list")]
    read_state: Vec<RawMentionState>,
}

impl MentionStatesPayload {
    /// The channels holding unread mentions. Guild-event read states (whose
    /// id isn't a channel) and channels without any are dropped.
    pub(super) fn into_mention_counts(self) -> Vec<(Id<ChannelMarker>, u32)> {
        self.read_state
            .into_iter()
            .filter(|state| state.read_state_type == 0)
            .map(|state| {
                (
                    state.id,
                    u32::try_from(state.mention_count.max(0)).unwrap_or(u32::MAX),
                )
            })
            .filter(|&(_, count)| count > 0)
            .collect()
    }
}

/// `STREAM_CREATE`, of which only what a connection needs is read.
#[derive(Deserialize)]
pub(super) struct StreamCreatePayload {
    pub(super) stream_key: String,
    pub(super) rtc_server_id: String,
}

/// `CALL_CREATE` and `CALL_UPDATE`: a DM's call, and who it's ringing. Only a
/// create carries who's already in it.
#[derive(Deserialize)]
pub(super) struct CallPayload {
    pub(super) channel_id: Id<ChannelMarker>,
    #[serde(default)]
    pub(super) ringing: Vec<Id<UserMarker>>,
    #[serde(default)]
    pub(super) voice_states: Vec<RawVoiceState>,
}

#[derive(Deserialize)]
pub(super) struct CallDeletePayload {
    pub(super) channel_id: Id<ChannelMarker>,
}

#[derive(Deserialize)]
pub(super) struct StreamDeletePayload {
    pub(super) stream_key: String,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

/// `USER_SETTINGS_PROTO_UPDATE`, of which only the `PreloadedUserSettings`
/// kind (type 1) is read. A partial update carries just the fields that
/// changed, so a missing status means it didn't change, not that it's gone.
#[derive(Deserialize)]
pub(super) struct SettingsProtoUpdatePayload {
    pub(super) settings: SettingsProtoUpdate,
}

#[derive(Deserialize)]
pub(super) struct SettingsProtoUpdate {
    #[serde(rename = "type")]
    pub(super) kind: u8,
    pub(super) proto: String,
}

#[derive(Deserialize)]
pub(super) struct ReadyPayload {
    pub(super) user: ReadyUser,
    /// A user session's guilds arrive whole in `READY`; a bot's only as ids,
    /// with the rest following in `GUILD_CREATE`.
    #[serde(default)]
    pub(super) guilds: Vec<GatewayGuild>,
    /// The signed-in user's member in each guild, in the same order as
    /// `guilds`, when the session asked for deduplicated payloads. Otherwise
    /// they arrive in each guild's own `members`.
    #[serde(default)]
    pub(super) merged_members: Vec<Vec<RawMember>>,
    /// A plain list, or versioned when the session asked for it.
    #[serde(default, deserialize_with = "versioned_list")]
    pub(super) read_state: Vec<RawReadState>,
    #[serde(default, deserialize_with = "versioned_list")]
    pub(super) user_guild_settings: Vec<RawGuildSettings>,
    /// DMs and group DMs, of which only how far each has got is read here —
    /// the DM list fetches the rest when it's opened.
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) private_channels: Vec<RawChannelUnread>,
    /// Friends' presences, unless the session asked for them merged.
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) presences: Vec<RawPresence>,
    #[serde(default)]
    pub(super) merged_presences: Option<MergedPresences>,
}

/// Presences gathered in one place, from `READY` or `READY_SUPPLEMENTAL`
/// depending on what the session asked for. `guilds` is in the same order as
/// `READY`'s guilds, which doesn't matter here: a status is the same in all.
#[derive(Deserialize)]
pub(super) struct MergedPresences {
    #[serde(default, deserialize_with = "skip_invalid")]
    friends: Vec<RawPresence>,
    #[serde(default)]
    guilds: Vec<Vec<Box<RawValue>>>,
}

impl MergedPresences {
    pub(super) fn into_presences(self) -> impl Iterator<Item = RawPresence> {
        let guilds = self.guilds.into_iter().flatten();
        self.friends
            .into_iter()
            .chain(guilds.filter_map(|presence| serde_json::from_str(presence.get()).ok()))
    }
}

/// A presence, of which only whose it is and their status are read. A
/// `PRESENCE_UPDATE` names the user in a partial `user`; a merged presence by
/// `user_id`.
#[derive(Deserialize)]
pub(super) struct RawPresence {
    #[serde(default)]
    user: Option<ReadyUser>,
    #[serde(default)]
    user_id: Option<Id<UserMarker>>,
    status: String,
}

/// The presences that name a user and a status the app knows.
pub(super) fn convert_presences(
    presences: impl IntoIterator<Item = RawPresence>,
) -> Vec<(Id<UserMarker>, PresenceStatus)> {
    presences
        .into_iter()
        .filter_map(|presence| {
            let user_id = presence.user_id.or(presence.user.map(|user| user.id))?;
            Some((user_id, PresenceStatus::from_presence(&presence.status)?))
        })
        .collect()
}

/// `READY_SUPPLEMENTAL`, of which only the presences are read.
#[derive(Deserialize)]
pub(super) struct ReadySupplementalPayload {
    #[serde(default)]
    pub(super) merged_presences: Option<MergedPresences>,
}

/// A guild as `READY` and `GUILD_CREATE` send it, of which only what's needed
/// is read: who is already sitting in its voice channels, what a role mention
/// should be called, and which roles the user holds. An unavailable guild
/// arrives with none of these.
#[derive(Deserialize)]
pub(super) struct GatewayGuild {
    pub(super) id: Id<GuildMarker>,
    /// Lenient because it sits inside `READY`: one state Discord shapes
    /// unexpectedly shouldn't cost the whole session its roles.
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) voice_states: Vec<RawVoiceState>,
    #[serde(default)]
    pub(super) roles: Vec<RawRole>,
    #[serde(default)]
    pub(super) members: Vec<RawMember>,
    /// `None` for an unavailable guild, which says nothing about its emoji,
    /// as opposed to a guild that has none.
    #[serde(default, deserialize_with = "skip_invalid_opt")]
    pub(super) emojis: Option<Vec<RawEmoji>>,
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) channels: Vec<RawGuildChannel>,
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) presences: Vec<RawPresence>,
    #[serde(default)]
    pub(super) joined_at: Option<Timestamp>,
    #[serde(default)]
    owner_id: Option<Id<UserMarker>>,
    /// Where the guild's own fields sit when the session asked for them
    /// apart from its lists.
    #[serde(default)]
    properties: Option<GuildProperties>,
}

#[derive(Deserialize)]
struct GuildProperties {
    #[serde(default)]
    owner_id: Option<Id<UserMarker>>,
}

impl GatewayGuild {
    pub(super) fn owner_id(&self) -> Option<Id<UserMarker>> {
        self.owner_id.or_else(|| self.properties.as_ref()?.owner_id)
    }
}

#[derive(Deserialize)]
pub(super) struct MessageAckPayload {
    pub(super) channel_id: Id<ChannelMarker>,
    pub(super) message_id: Id<MessageMarker>,
    #[serde(default)]
    pub(super) manual: bool,
}

/// `CHANNEL_UNREAD_UPDATE`, `PASSIVE_UPDATE_V1`, and `PASSIVE_UPDATE_V2`,
/// which each carry channels' newest messages under a name of their own.
#[derive(Deserialize)]
pub(super) struct ChannelUnreadsPayload {
    #[serde(
        default,
        alias = "channels",
        alias = "updated_channels",
        deserialize_with = "skip_invalid"
    )]
    pub(super) channel_unread_updates: Vec<RawChannelUnread>,
}

/// `GUILD_EMOJIS_UPDATE`: the guild's whole emoji list as it now stands.
#[derive(Deserialize)]
pub(super) struct EmojisUpdatePayload {
    pub(super) guild_id: Id<GuildMarker>,
    #[serde(deserialize_with = "skip_invalid")]
    pub(super) emojis: Vec<RawEmoji>,
}

/// `CHANNEL_CREATE`, `CHANNEL_UPDATE`, and `CHANNEL_DELETE`, of which only
/// the guild is read. A DM's has none.
#[derive(Deserialize)]
pub(super) struct ChannelChangePayload {
    #[serde(default)]
    pub(super) guild_id: Option<Id<GuildMarker>>,
}

/// `GUILD_DELETE`: the user left the guild, or it went unavailable.
#[derive(Deserialize)]
pub(super) struct GuildDeletePayload {
    pub(super) id: Id<GuildMarker>,
}

/// `GUILD_MEMBER_UPDATE`.
#[derive(Deserialize)]
pub(super) struct MemberUpdatePayload {
    pub(super) guild_id: Id<GuildMarker>,
    #[serde(flatten)]
    pub(super) member: RawMember,
}

/// `GUILD_MEMBERS_CHUNK`, the answer to [`GatewaySender::request_members`](super::GatewaySender::request_members).
#[derive(Deserialize)]
pub(super) struct MembersChunkPayload {
    pub(super) guild_id: Id<GuildMarker>,
    #[serde(default, deserialize_with = "skip_invalid")]
    pub(super) members: Vec<RawMember>,
}

/// `GUILD_ROLE_CREATE` and `GUILD_ROLE_UPDATE`.
#[derive(Deserialize)]
pub(super) struct RoleUpdatePayload {
    pub(super) guild_id: Id<GuildMarker>,
    pub(super) role: RawRole,
}

#[derive(Deserialize)]
pub(super) struct RoleDeletePayload {
    pub(super) guild_id: Id<GuildMarker>,
    pub(super) role_id: Id<RoleMarker>,
}

/// The `nonce` on a `MESSAGE_CREATE`. Discord echoes it as whatever the sender
/// used, a string or a number, and other clients' nonces needn't be numeric.
#[derive(Deserialize)]
pub(super) struct NoncePayload {
    #[serde(default, deserialize_with = "deserialize_nonce")]
    pub(super) nonce: Option<u64>,
}

fn deserialize_nonce<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::String(nonce)) => nonce.parse().ok(),
            Some(serde_json::Value::Number(nonce)) => nonce.as_u64(),
            _ => None,
        },
    )
}

#[derive(Deserialize)]
pub(super) struct MessageDeletePayload {
    pub(super) id: Id<MessageMarker>,
    pub(super) channel_id: Id<ChannelMarker>,
}

#[derive(Deserialize)]
pub(super) struct MessageDeleteBulkPayload {
    pub(super) ids: Vec<Id<MessageMarker>>,
    pub(super) channel_id: Id<ChannelMarker>,
}

/// `MESSAGE_REACTION_ADD`, `MESSAGE_REACTION_REMOVE`, and the two clears,
/// which share a shape: the clears just leave out who reacted, and clearing
/// all of a message's reactions leaves out the emoji too.
#[derive(Deserialize)]
pub(super) struct ReactionPayload {
    pub(super) channel_id: Id<ChannelMarker>,
    pub(super) message_id: Id<MessageMarker>,
    #[serde(default)]
    pub(super) user_id: Option<Id<UserMarker>>,
    #[serde(default)]
    pub(super) emoji: Option<twilight_model::channel::message::EmojiReactionType>,
}

#[derive(Deserialize)]
pub(super) struct TypingStartPayload {
    pub(super) channel_id: Id<ChannelMarker>,
    pub(super) user_id: Id<UserMarker>,
    #[serde(default)]
    pub(super) member: Option<RawMember>,
}

#[derive(Deserialize)]
pub(super) struct ReadyUser {
    pub(super) id: Id<UserMarker>,
}

/// Deserializes a list item by item, dropping the items that don't fit `T`
/// instead of failing the whole payload. The items are borrowed from the frame
/// first, so the skipping costs no copies.
fn skip_invalid<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let items = Vec::<&'de RawValue>::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_str(item.get()).ok())
        .collect())
}

fn skip_invalid_opt<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    skip_invalid(deserializer).map(Some)
}
