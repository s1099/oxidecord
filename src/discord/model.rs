//! The app's view of Discord's data, plus the conversions from twilight's API
//! models that build it.
//!
//! Everything the UI renders goes through these types, so the twilight models
//! stay confined to this module and [`super::rest`]/[`super::gateway`].

use serde::Deserialize;
use twilight_model::channel::ChannelType;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, EmojiMarker, GuildMarker, MessageMarker, UserMarker},
};
use twilight_model::util::Timestamp;

#[derive(Clone)]
pub struct Guild {
    pub id: Id<GuildMarker>,
    pub name: String,
    pub icon_url: Option<String>,
    /// The user's guild-wide permissions (from `@everyone` plus their roles,
    /// before any channel overwrites). Discord hands these to us with the
    /// guild list, so channel visibility can be resolved without refetching
    /// the guild's roles.
    pub permissions: Permissions,
    /// Whether the current user owns this guild (owners bypass permissions).
    pub owner: bool,
}

#[derive(Clone)]
pub struct CurrentUser {
    /// Display name: the global display name when set, else the username.
    pub name: String,
    /// The `@handle` username.
    pub username: String,
    pub avatar_url: Option<String>,
}

/// Another user, as shown in the profile popout opened from their avatar.
#[derive(Clone)]
pub struct UserProfile {
    /// Display name: the global display name when set, else the username.
    pub name: String,
    /// The `@handle` username.
    pub username: String,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    /// The banner's solid colour, used when the user has no banner image.
    /// Packed as `0xRRGGBB`.
    pub accent_color: Option<u32>,
    /// The "About Me" text. `None` when unset or when only the fallback
    /// `GET /users/{id}` data was available, which doesn't carry a bio.
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    /// When the account was registered, derived from the snowflake and
    /// formatted as `Jan 5, 2021`.
    pub created_at: String,
    pub bot: bool,
}

/// `GET /users/{id}/profile`, the endpoint the Discord client itself uses for
/// the profile popout. It has no twilight model, so it's deserialized here.
#[derive(Deserialize)]
pub(super) struct RawProfile {
    user: RawProfileUser,
    /// The user's global profile, whose banner/bio/accent override the ones on
    /// `user` (which are the per-guild values when a guild was requested).
    #[serde(default)]
    user_profile: Option<RawProfileDetails>,
}

#[derive(Deserialize)]
struct RawProfileUser {
    id: Id<UserMarker>,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    accent_color: Option<u32>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    bot: bool,
}

#[derive(Deserialize)]
struct RawProfileDetails {
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    accent_color: Option<u32>,
    #[serde(default)]
    pronouns: Option<String>,
}

impl RawProfile {
    pub(super) fn into_profile(self) -> UserProfile {
        let details = self.user_profile;
        let user = self.user;

        // The global profile wins wherever it has a value; `user` is the
        // fallback for accounts that only set the fields in one place.
        let banner = details
            .as_ref()
            .and_then(|details| details.banner.clone())
            .or(user.banner);
        let bio = details
            .as_ref()
            .and_then(|details| details.bio.clone())
            .or(user.bio);
        let accent_color = details
            .as_ref()
            .and_then(|details| details.accent_color)
            .or(user.accent_color);

        UserProfile {
            name: user.global_name.unwrap_or_else(|| user.username.clone()),
            username: user.username,
            avatar_url: user
                .avatar
                .as_deref()
                .map(|hash| cdn_avatar_url(user.id.get(), hash, 160)),
            banner_url: banner
                .as_deref()
                .map(|hash| cdn_banner_url(user.id.get(), hash, 480)),
            accent_color,
            bio: bio.filter(|bio| !bio.trim().is_empty()),
            pronouns: details
                .and_then(|details| details.pronouns)
                .filter(|pronouns| !pronouns.trim().is_empty()),
            created_at: format_snowflake_date(user.id.get()),
            bot: user.bot,
        }
    }
}

/// Builds a [`UserProfile`] from the plain `GET /users/{id}` user, used when
/// the richer profile endpoint isn't available to this token. The bio and
/// pronouns aren't part of that response, so they're left unset.
pub(super) fn convert_user_profile(user: twilight_model::user::User) -> UserProfile {
    UserProfile {
        name: user
            .global_name
            .clone()
            .unwrap_or_else(|| user.name.clone()),
        username: user.name,
        avatar_url: user
            .avatar
            .map(|hash| cdn_avatar_url(user.id.get(), &hash.to_string(), 160)),
        banner_url: user
            .banner
            .map(|hash| cdn_banner_url(user.id.get(), &hash.to_string(), 480)),
        accent_color: user.accent_color,
        bio: None,
        pronouns: None,
        created_at: format_snowflake_date(user.id.get()),
        bot: user.bot,
    }
}

/// Extension for a CDN image hash: animated assets are prefixed `a_` and only
/// animate as GIF, while static ones are smaller as webp.
fn cdn_extension(hash: &str) -> &'static str {
    if hash.starts_with("a_") {
        "gif"
    } else {
        "webp"
    }
}

fn cdn_avatar_url(user_id: u64, hash: &str, size: u32) -> String {
    format!(
        "https://cdn.discordapp.com/avatars/{user_id}/{hash}.{}?size={size}",
        cdn_extension(hash)
    )
}

fn cdn_banner_url(user_id: u64, hash: &str, size: u32) -> String {
    format!(
        "https://cdn.discordapp.com/banners/{user_id}/{hash}.{}?size={size}",
        cdn_extension(hash)
    )
}

/// The subset of Discord channel types the app knows how to display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Announcement,
    Voice,
    Stage,
    Forum,
    Category,
}

impl ChannelKind {
    pub fn is_voice(self) -> bool {
        matches!(self, Self::Voice | Self::Stage)
    }
}

#[derive(Clone)]
pub struct Channel {
    pub id: Id<ChannelMarker>,
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<Id<ChannelMarker>>,
    pub position: i32,
    pub topic: Option<String>,
}

/// A 1:1 or group direct-message conversation from the user's DM list.
#[derive(Clone)]
pub struct DirectMessage {
    pub id: Id<ChannelMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// Snowflake of the last message, used only to order conversations by
    /// recency like Discord does. `None` (no messages yet) sorts last.
    last_message_id: Option<u64>,
}

#[derive(Clone)]
pub struct Message {
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub content: String,
    pub timestamp: String,
    pub images: Vec<ImageAttachment>,
    /// The message this one is a reply to, when it references another. Carries
    /// just enough to render the quoted preview above the message.
    pub reply: Option<MessageReference>,
    /// Reactions on the message, in Discord's order (first reacted first).
    pub reactions: Vec<Reaction>,
}

/// One emoji's reaction tally on a message.
#[derive(Clone)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    pub count: u64,
    /// Whether the current user is one of the reactors.
    pub me: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ReactionEmoji {
    Unicode(String),
    Custom {
        id: Id<EmojiMarker>,
        /// Empty when the emoji has been deleted from its guild.
        name: String,
        animated: bool,
    },
}

impl ReactionEmoji {
    /// CDN URL for a custom emote's image; `None` for unicode emoji, which are
    /// rendered as text.
    pub fn image_url(&self) -> Option<String> {
        match self {
            Self::Unicode(_) => None,
            // Animated emotes only animate as GIF; static ones are smaller as
            // webp.
            Self::Custom { id, animated, .. } => Some(format!(
                "https://cdn.discordapp.com/emojis/{id}.{}?size=44",
                if *animated { "gif" } else { "webp" }
            )),
        }
    }
}

/// A compact snapshot of the message a reply points at, used to render the
/// quoted preview line atop the reply.
#[derive(Clone)]
pub struct MessageReference {
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    /// A single-line preview of the referenced message's content. Empty when
    /// the original had no text (e.g. an attachment-only message).
    pub content: String,
}

#[derive(Clone)]
pub struct ImageAttachment {
    pub url: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// inline preview while preserving aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub(super) fn convert_guild(guild: twilight_model::user::CurrentUserGuild) -> Guild {
    let icon_url = guild.icon.map(|hash| {
        format!(
            "https://cdn.discordapp.com/icons/{}/{}.webp?size=100&quality=lossless",
            guild.id, hash
        )
    });
    Guild {
        id: guild.id,
        name: guild.name,
        icon_url,
        permissions: guild.permissions,
        owner: guild.owner,
    }
}

pub(super) fn convert_current_user(user: twilight_model::user::CurrentUser) -> CurrentUser {
    let avatar_url = user.avatar.map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{}/{}.webp?size=80",
            user.id, hash
        )
    });
    let name = user
        .global_name
        .clone()
        .unwrap_or_else(|| user.name.clone());
    CurrentUser {
        name,
        username: user.name,
        avatar_url,
    }
}

/// Converts a guild channel, dropping the types the UI can't display (threads,
/// directories, ...).
pub(super) fn convert_channel(channel: twilight_model::channel::Channel) -> Option<Channel> {
    let kind = match channel.kind {
        ChannelType::GuildText => ChannelKind::Text,
        ChannelType::GuildAnnouncement => ChannelKind::Announcement,
        ChannelType::GuildVoice => ChannelKind::Voice,
        ChannelType::GuildStageVoice => ChannelKind::Stage,
        ChannelType::GuildForum => ChannelKind::Forum,
        ChannelType::GuildCategory => ChannelKind::Category,
        _ => return None,
    };

    Some(Channel {
        id: channel.id,
        name: channel.name.unwrap_or_default(),
        kind,
        parent_id: channel.parent_id,
        position: channel.position.unwrap_or(0),
        topic: channel.topic.filter(|topic| !topic.is_empty()),
    })
}

/// Converts the raw private-channel list into DM conversations, ordered
/// most-recently-active first.
pub(super) fn convert_dms(channels: Vec<twilight_model::channel::Channel>) -> Vec<DirectMessage> {
    let mut dms: Vec<DirectMessage> = channels.into_iter().filter_map(convert_dm).collect();
    dms.sort_by_key(|dm| std::cmp::Reverse(dm.last_message_id));
    dms
}

fn convert_dm(channel: twilight_model::channel::Channel) -> Option<DirectMessage> {
    let last_message_id = channel.last_message_id.map(|id| id.get());
    match channel.kind {
        ChannelType::Private => {
            let user = channel.recipients?.into_iter().next()?;
            let avatar_url = user.avatar.map(|hash| {
                format!(
                    "https://cdn.discordapp.com/avatars/{}/{}.webp?size=80",
                    user.id, hash
                )
            });
            Some(DirectMessage {
                id: channel.id,
                name: user.global_name.unwrap_or(user.name),
                avatar_url,
                last_message_id,
            })
        }
        ChannelType::Group => {
            let avatar_url = channel.icon.map(|hash| {
                format!(
                    "https://cdn.discordapp.com/channel-icons/{}/{}.webp?size=80",
                    channel.id, hash
                )
            });
            let name = channel
                .name
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    let names = channel.recipients.as_ref()?;
                    let joined = names
                        .iter()
                        .map(|user| {
                            user.global_name
                                .clone()
                                .unwrap_or_else(|| user.name.clone())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    (!joined.is_empty()).then_some(joined)
                })
                .unwrap_or_else(|| "Group DM".to_string());
            Some(DirectMessage {
                id: channel.id,
                name,
                avatar_url,
                last_message_id,
            })
        }
        _ => None,
    }
}

pub(super) fn convert_message(message: twilight_model::channel::Message) -> Message {
    let author_avatar_url = avatar_url(&message.author);
    // Prefer the author's per-guild nickname, then their global display name,
    // then their username. `member` is present on messages fetched from a
    // guild channel but not on ones we just sent, hence the fallbacks.
    let author_name = message
        .member
        .and_then(|member| member.nick)
        .or(message.author.global_name)
        .unwrap_or_else(|| message.author.name.clone());

    let images = message
        .attachments
        .iter()
        .filter(|attachment| is_image_attachment(attachment))
        .map(|attachment| ImageAttachment {
            url: preview_image_url(attachment),
            width: attachment.width.map(|w| w as u32),
            height: attachment.height.map(|h| h as u32),
        })
        .collect();

    Message {
        id: message.id,
        author_id: message.author.id,
        author_name,
        author_avatar_url,
        content: message.content,
        timestamp: format_timestamp(message.timestamp),
        images,
        reply: message
            .referenced_message
            .map(|referenced| convert_reference(*referenced)),
        reactions: message
            .reactions
            .into_iter()
            .map(|reaction| Reaction {
                emoji: convert_emoji(reaction.emoji),
                count: reaction.count,
                me: reaction.me,
            })
            .collect(),
    }
}

fn convert_emoji(emoji: twilight_model::channel::message::EmojiReactionType) -> ReactionEmoji {
    use twilight_model::channel::message::EmojiReactionType;
    match emoji {
        EmojiReactionType::Unicode { name } => ReactionEmoji::Unicode(name),
        EmojiReactionType::Custom { id, name, animated } => ReactionEmoji::Custom {
            id,
            name: name.unwrap_or_default(),
            animated,
        },
    }
}

/// Extracts the referenced message into a compact preview. Referenced messages
/// don't carry guild `member` data, so the author name falls back to the global
/// display name and then the username.
fn convert_reference(referenced: twilight_model::channel::Message) -> MessageReference {
    let author_name = referenced
        .author
        .global_name
        .clone()
        .unwrap_or_else(|| referenced.author.name.clone());
    MessageReference {
        author_name,
        author_avatar_url: avatar_url(&referenced.author),
        content: single_line_preview(&referenced.content),
    }
}

fn avatar_url(user: &twilight_model::user::User) -> Option<String> {
    user.avatar.map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{}/{}.webp?size=80",
            user.id, hash
        )
    })
}

/// Condense a message's content into a single line preview for the reply quote.
fn single_line_preview(content: &str) -> String {
    const MAX_LEN: usize = 150;

    let first_line = content.lines().next().unwrap_or_default().trim_end();
    let has_more_lines = content.lines().nth(1).is_some();

    let truncated: String = first_line.chars().take(MAX_LEN).collect();
    let is_clipped = truncated.chars().count() < first_line.chars().count();

    if is_clipped || has_more_lines {
        format!("{}…", truncated.trim_end())
    } else {
        truncated
    }
}

/// Formats a Discord timestamp as `YYYY-MM-DD HH:MM` (UTC). Its ISO 8601 form
/// is `2021-08-10T11:16:37.020000+00:00`.
fn format_timestamp(timestamp: Timestamp) -> String {
    let iso = timestamp.iso_8601().to_string();
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => iso,
    }
}

/// Milliseconds between the Unix epoch and Discord's (2015-01-01), the offset
/// the timestamp inside a snowflake is measured from.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Formats the creation time encoded in a snowflake as `Jan 5, 2021` (UTC),
/// the form Discord uses for "Member Since".
fn format_snowflake_date(id: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    // The upper 42 bits are milliseconds since the Discord epoch.
    let unix_ms = (id >> 22) + DISCORD_EPOCH_MS;
    let (year, month, day) = civil_from_days((unix_ms / 86_400_000) as i64);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// Converts days since the Unix epoch into a `(year, month, day)` civil date,
/// via Howard Hinnant's `civil_from_days` algorithm. Avoids pulling in a date
/// library for the one date the app formats this way.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the era to start on 0000-03-01, so the leap day lands at the end of
    // the year and every era is exactly 146097 days.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // March-based month index (0 = March … 11 = February).
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

const PREVIEW_MAX_WIDTH: u32 = 480;
const PREVIEW_MAX_HEIGHT: u32 = 390;

fn preview_image_url(attachment: &twilight_model::channel::Attachment) -> String {
    let (target_w, target_h) = match (attachment.width, attachment.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let scale = (PREVIEW_MAX_WIDTH as f64 / w as f64)
                .min(PREVIEW_MAX_HEIGHT as f64 / h as f64)
                .min(1.0);
            (
                (w as f64 * scale).round().max(1.0) as u32,
                (h as f64 * scale).round().max(1.0) as u32,
            )
        }
        _ => (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT),
    };
    // proxy_url already carries a signed query string, so append with `&`.
    let separator = if attachment.proxy_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{}{separator}width={target_w}&height={target_h}",
        attachment.proxy_url
    )
}

/// Whether an attachment is an image we can render inline. Prefers Discord's
/// reported media type and falls back to the filename extension.
fn is_image_attachment(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("image/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}
