//! Read states: how far the user has read each channel, how far each channel
//! has got, and which are muted or set to mark only pings unread — what the
//! unread marks are worked out from.
//!
//! None of this is in twilight, since bots have no read state. Discord's own
//! client works the marks out itself from the same three things: a channel is
//! unread when its newest message is past the one the user last acked.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use twilight_model::channel::ChannelType;
use twilight_model::channel::permission_overwrite::PermissionOverwrite;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};
use twilight_model::util::Timestamp;
use twilight_util::permission_calculator::PermissionCalculator;

use super::RawRole;
use super::channel::{Channel, GuildChannelFields, guild_channel};

/// Milliseconds from the Unix epoch to Discord's, which snowflakes count from.
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

pub struct ReadState {
    pub channel_id: Id<ChannelMarker>,
    pub last_message_id: Id<MessageMarker>,
    /// How many unread messages ping the user.
    pub mention_count: u32,
}

/// The newest message in a channel.
pub struct ChannelLatest {
    pub channel_id: Id<ChannelMarker>,
    pub last_message_id: Id<MessageMarker>,
}

/// A guild's channels as far as its unread mark goes: the ones the user can
/// see that hold messages the app can show.
pub struct GuildUnreads {
    pub guild_id: Id<GuildMarker>,
    /// When the user joined, as a snowflake. A channel they've never opened
    /// has no read state, and Discord counts it read up to here.
    pub joined: Option<Id<MessageMarker>>,
    pub channels: Vec<UnreadChannel>,
}

pub struct UnreadChannel {
    pub id: Id<ChannelMarker>,
    /// The category, whose mute carries down to the channel.
    pub parent_id: Option<Id<ChannelMarker>>,
    pub last_message_id: Option<Id<MessageMarker>>,
}

/// Whether new messages mark a channel unread, or only messages that ping
/// the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnreadSetting {
    AllMessages,
    OnlyMentions,
}

impl UnreadSetting {
    /// What a notification level means for unreads when no unread setting
    /// was picked apart from it: only "all messages" marks every message
    /// unread. `None` for the level that inherits from the guild.
    fn from_notifications(level: u8) -> Option<Self> {
        match level {
            0 => Some(Self::AllMessages),
            1 | 2 => Some(Self::OnlyMentions),
            _ => None,
        }
    }

    /// The setting picked by `all` or `only_mentions` in `flags`, or else the
    /// one `notifications` implies.
    fn resolve(
        flags: u64,
        all: u64,
        only_mentions: u64,
        notifications: Option<u8>,
    ) -> Option<Self> {
        if flags & all != 0 {
            Some(Self::AllMessages)
        } else if flags & only_mentions != 0 {
            Some(Self::OnlyMentions)
        } else {
            notifications.and_then(Self::from_notifications)
        }
    }

    /// What a guild's `default_message_notifications` means for unreads.
    pub(in crate::discord) fn guild_default(level: u8) -> Self {
        Self::from_notifications(level).unwrap_or(Self::AllMessages)
    }
}

/// What the user has muted in one guild, or across their DMs when `guild_id`
/// is `None`, and what they've set to mark it unread.
pub struct MuteSettings {
    pub guild_id: Option<Id<GuildMarker>>,
    pub muted: bool,
    pub muted_channels: Vec<Id<ChannelMarker>>,
    /// What marks the guild's channels unread, unless one says otherwise.
    /// `None` leaves it to the guild's default notifications.
    pub unreads: Option<UnreadSetting>,
    /// Channels and categories with a setting of their own.
    pub channel_unreads: Vec<(Id<ChannelMarker>, UnreadSetting)>,
    /// Whether `@everyone` and `@here` don't count as pinging the user.
    pub suppress_everyone: bool,
    /// Whether mentions of the user's roles don't count as pinging them.
    pub suppress_roles: bool,
}

/// One entry of `READY`'s `read_state`. Only channel read states (type 0) are
/// kept; the rest are guild events, the notification center, and the like,
/// whose `id` isn't a channel.
#[derive(Deserialize)]
pub(in crate::discord) struct RawReadState {
    id: Id<ChannelMarker>,
    #[serde(default)]
    read_state_type: u8,
    #[serde(default, deserialize_with = "snowflake")]
    last_message_id: Option<Id<MessageMarker>>,
    #[serde(default)]
    mention_count: Option<u32>,
}

/// A channel's newest message, as `CHANNEL_UNREAD_UPDATE` and the passive
/// updates send it.
#[derive(Deserialize)]
pub(in crate::discord) struct RawChannelUnread {
    id: Id<ChannelMarker>,
    #[serde(default, deserialize_with = "snowflake")]
    last_message_id: Option<Id<MessageMarker>>,
}

/// A channel in a gateway guild, of which only what the sidebar shows and
/// what decides its unread mark is read.
#[derive(Deserialize)]
pub(in crate::discord) struct RawGuildChannel {
    id: Id<ChannelMarker>,
    #[serde(rename = "type")]
    kind: ChannelType,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<i32>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    parent_id: Option<Id<ChannelMarker>>,
    #[serde(default, deserialize_with = "snowflake")]
    last_message_id: Option<Id<MessageMarker>>,
    #[serde(default)]
    permission_overwrites: Vec<PermissionOverwrite>,
}

/// One entry of `user_guild_settings`, or a `USER_GUILD_SETTINGS_UPDATE`.
#[derive(Deserialize)]
pub(in crate::discord) struct RawGuildSettings {
    #[serde(default)]
    guild_id: Option<Id<GuildMarker>>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    mute_config: Option<RawMuteConfig>,
    #[serde(default)]
    suppress_everyone: bool,
    #[serde(default)]
    suppress_roles: bool,
    #[serde(default)]
    message_notifications: Option<u8>,
    #[serde(default)]
    flags: Option<u64>,
    #[serde(default, deserialize_with = "versioned_list")]
    channel_overrides: Vec<RawChannelOverride>,
}

#[derive(Deserialize)]
struct RawChannelOverride {
    channel_id: Id<ChannelMarker>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    mute_config: Option<RawMuteConfig>,
    #[serde(default)]
    message_notifications: Option<u8>,
    #[serde(default)]
    flags: Option<u64>,
}

/// `UNREADS_ALL_MESSAGES` and `UNREADS_ONLY_MENTIONS` in a guild's settings'
/// flags.
const GUILD_UNREADS_ALL: u64 = 1 << 11;
const GUILD_UNREADS_ONLY_MENTIONS: u64 = 1 << 12;
/// The same in a channel override's flags, which number them the other way
/// round.
const CHANNEL_UNREADS_ONLY_MENTIONS: u64 = 1 << 9;
const CHANNEL_UNREADS_ALL: u64 = 1 << 10;

#[derive(Deserialize)]
struct RawMuteConfig {
    #[serde(default)]
    end_time: Option<Timestamp>,
}

/// Reads an id Discord may send as `0` or `"0"` for "none", which `Id`
/// itself refuses.
fn snowflake<'de, D, T>(deserializer: D) -> Result<Option<Id<T>>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::String(id)) => id.parse().ok().and_then(Id::new_checked),
            Some(serde_json::Value::Number(id)) => id.as_u64().and_then(Id::new_checked),
            _ => None,
        },
    )
}

/// Reads a list that may come as a plain array, as a versioned
/// `{entries, partial, version}` object, or keyed by id, depending on the
/// session's capabilities. Items that don't fit are dropped.
pub(in crate::discord) fn versioned_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let items = match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(mut object) => match object.remove("entries") {
            Some(serde_json::Value::Array(items)) => items,
            _ => object.into_iter().map(|(_, item)| item).collect(),
        },
        _ => Vec::new(),
    };
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

pub(in crate::discord) fn convert_read_states(states: Vec<RawReadState>) -> Vec<ReadState> {
    states
        .into_iter()
        .filter(|state| state.read_state_type == 0)
        .filter_map(|state| {
            Some(ReadState {
                channel_id: state.id,
                last_message_id: state.last_message_id?,
                mention_count: state.mention_count.unwrap_or(0),
            })
        })
        .collect()
}

pub(in crate::discord) fn convert_channel_unreads(
    channels: Vec<RawChannelUnread>,
) -> Vec<ChannelLatest> {
    channels
        .into_iter()
        .filter_map(|channel| {
            Some(ChannelLatest {
                channel_id: channel.id,
                last_message_id: channel.last_message_id?,
            })
        })
        .collect()
}

/// A guild's channels from a gateway guild: the sidebar's list, and the ones
/// that count toward its unread mark — text and announcement channels the
/// user can view. Voice channels' text chats and forum posts are left out of
/// the latter, since the app can't open them to read them.
///
/// This is how Discord's own client gets a guild's channels: they all come
/// with `READY`, so opening a guild needs no request. Visibility is worked
/// out from the roles in the same payload. `member_roles` is `None` when the
/// user's member didn't come with it, which leaves only what `@everyone` can
/// see.
pub(in crate::discord) fn convert_guild_channels(
    guild_id: Id<GuildMarker>,
    user_id: Id<UserMarker>,
    owner_id: Option<Id<UserMarker>>,
    joined_at: Option<Timestamp>,
    roles: &[RawRole],
    member_roles: Option<&[Id<RoleMarker>]>,
    channels: Vec<RawGuildChannel>,
) -> (GuildUnreads, Vec<Channel>) {
    let role_permissions = |id: Id<RoleMarker>| {
        roles
            .iter()
            .find(|role| role.id == id)
            .and_then(|role| role.permissions)
            .unwrap_or_else(Permissions::empty)
    };
    // `@everyone` shares the guild's id.
    let everyone = role_permissions(guild_id.cast());
    let member_roles: Vec<(Id<RoleMarker>, Permissions)> = member_roles
        .unwrap_or_default()
        .iter()
        .filter(|id| id.get() != guild_id.get())
        .map(|&id| (id, role_permissions(id)))
        .collect();

    let mut unread_channels = Vec::new();
    let mut visible = Vec::new();
    for channel in channels {
        let mut calculator = PermissionCalculator::new(guild_id, user_id, everyone, &member_roles);
        if owner_id == Some(user_id) {
            calculator = calculator.owner_id(user_id);
        }
        let permissions = calculator.in_channel(channel.kind, &channel.permission_overwrites);
        if !permissions.contains(Permissions::VIEW_CHANNEL) {
            continue;
        }
        if matches!(
            channel.kind,
            ChannelType::GuildText | ChannelType::GuildAnnouncement
        ) {
            unread_channels.push(UnreadChannel {
                id: channel.id,
                parent_id: channel.parent_id,
                last_message_id: channel.last_message_id,
            });
        }
        visible.extend(guild_channel(GuildChannelFields {
            id: channel.id,
            kind: channel.kind,
            name: channel.name,
            parent_id: channel.parent_id,
            position: channel.position,
            topic: channel.topic,
            last_message_id: channel.last_message_id,
            permissions,
        }));
    }

    let unreads = GuildUnreads {
        guild_id,
        joined: joined_at.and_then(timestamp_snowflake),
        channels: unread_channels,
    };
    (unreads, visible)
}

/// The lowest snowflake minted at `timestamp`.
fn timestamp_snowflake(timestamp: Timestamp) -> Option<Id<MessageMarker>> {
    let ms = timestamp.as_micros() / 1000 - DISCORD_EPOCH_MS;
    u64::try_from(ms)
        .ok()
        .and_then(|ms| Id::new_checked(ms << 22))
}

/// What's muted, with mutes whose time ran out already dropped. One that
/// runs out during the session stays in force until the settings next change.
pub(in crate::discord) fn convert_mute_settings(settings: RawGuildSettings) -> MuteSettings {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    let in_force = |muted: bool, config: Option<&RawMuteConfig>| {
        muted
            && config
                .and_then(|config| config.end_time)
                .is_none_or(|end| end.as_secs() > now)
    };

    MuteSettings {
        guild_id: settings.guild_id,
        muted: in_force(settings.muted, settings.mute_config.as_ref()),
        muted_channels: settings
            .channel_overrides
            .iter()
            .filter(|channel| in_force(channel.muted, channel.mute_config.as_ref()))
            .map(|channel| channel.channel_id)
            .collect(),
        unreads: UnreadSetting::resolve(
            settings.flags.unwrap_or(0),
            GUILD_UNREADS_ALL,
            GUILD_UNREADS_ONLY_MENTIONS,
            settings.message_notifications,
        ),
        channel_unreads: settings
            .channel_overrides
            .iter()
            .filter_map(|channel| {
                let unreads = UnreadSetting::resolve(
                    channel.flags.unwrap_or(0),
                    CHANNEL_UNREADS_ALL,
                    CHANNEL_UNREADS_ONLY_MENTIONS,
                    channel.message_notifications,
                )?;
                Some((channel.channel_id, unreads))
            })
            .collect(),
        suppress_everyone: settings.suppress_everyone,
        suppress_roles: settings.suppress_roles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_states_come_plain_or_versioned_and_skip_other_kinds() {
        #[derive(Deserialize)]
        struct Ready {
            #[serde(deserialize_with = "versioned_list")]
            read_state: Vec<RawReadState>,
        }

        let plain: Ready = serde_json::from_str(
            r#"{"read_state": [
                {"id": "100", "last_message_id": "500"},
                {"id": "101", "last_message_id": 0},
                {"id": "1", "read_state_type": 2, "last_acked_id": "7"}
            ]}"#,
        )
        .unwrap();
        let states = convert_read_states(plain.read_state);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].last_message_id, Id::new(500));

        let versioned: Ready = serde_json::from_str(
            r#"{"read_state": {"entries": [{"id": "100", "last_message_id": "500"}],
                "partial": false, "version": 3}}"#,
        )
        .unwrap();
        assert_eq!(convert_read_states(versioned.read_state).len(), 1);
    }

    #[test]
    fn joining_converts_to_a_snowflake() {
        // 2015-01-01T00:00:01Z is one second past Discord's epoch.
        let joined = Timestamp::parse("2015-01-01T00:00:01.000000+00:00").unwrap();
        assert_eq!(timestamp_snowflake(joined), Id::new_checked(1000 << 22));
    }

    #[test]
    fn hidden_channels_are_left_out() {
        let roles: Vec<RawRole> =
            serde_json::from_str(r#"[{"id": "10", "name": "@everyone", "permissions": "1024"}]"#)
                .unwrap();
        let channels: Vec<RawGuildChannel> = serde_json::from_str(
            r#"[
                {"id": "100", "type": 0, "last_message_id": "5"},
                {"id": "101", "type": 0, "permission_overwrites": [
                    {"id": "10", "type": 0, "allow": "0", "deny": "1024"}
                ]},
                {"id": "102", "type": 2},
                {"id": "103", "type": 4}
            ]"#,
        )
        .unwrap();

        let (unreads, visible) =
            convert_guild_channels(Id::new(10), Id::new(1), None, None, &roles, None, channels);
        let ids: Vec<_> = unreads.channels.iter().map(|channel| channel.id).collect();
        assert_eq!(ids, [Id::new(100)]);
        // The sidebar keeps the voice channel and the category, but not the
        // hidden channel.
        let ids: Vec<_> = visible.iter().map(|channel| channel.id).collect();
        assert_eq!(ids, [Id::new(100), Id::new(102), Id::new(103)]);
    }

    #[test]
    fn expired_mutes_are_dropped() {
        let settings: RawGuildSettings = serde_json::from_str(
            r#"{
                "guild_id": "10",
                "muted": true,
                "mute_config": {"end_time": "2000-01-01T00:00:00+00:00"},
                "channel_overrides": [
                    {"channel_id": "100", "muted": true, "mute_config": null},
                    {"channel_id": "101", "muted": false}
                ]
            }"#,
        )
        .unwrap();
        let mutes = convert_mute_settings(settings);
        assert!(!mutes.muted);
        assert_eq!(mutes.muted_channels, [Id::new(100)]);
    }

    #[test]
    fn unread_settings_follow_flags_then_notifications() {
        let settings: RawGuildSettings = serde_json::from_str(
            r#"{
                "guild_id": "10",
                "message_notifications": 1,
                "channel_overrides": [
                    {"channel_id": "100", "message_notifications": 3, "flags": 1024},
                    {"channel_id": "101", "message_notifications": 2},
                    {"channel_id": "102", "message_notifications": 3, "flags": null}
                ]
            }"#,
        )
        .unwrap();
        let settings = convert_mute_settings(settings);
        assert_eq!(settings.unreads, Some(UnreadSetting::OnlyMentions));
        assert_eq!(
            settings.channel_unreads,
            [
                (Id::new(100), UnreadSetting::AllMessages),
                (Id::new(101), UnreadSetting::OnlyMentions),
            ]
        );

        // The flag outranks the notification level, and a level that
        // inherits leaves it to the guild's default.
        let settings: RawGuildSettings = serde_json::from_str(
            r#"{"guild_id": "10", "message_notifications": 1, "flags": 2048}"#,
        )
        .unwrap();
        assert_eq!(
            convert_mute_settings(settings).unreads,
            Some(UnreadSetting::AllMessages)
        );
        let settings: RawGuildSettings =
            serde_json::from_str(r#"{"guild_id": "10", "message_notifications": 3}"#).unwrap();
        assert_eq!(convert_mute_settings(settings).unreads, None);
    }
}
