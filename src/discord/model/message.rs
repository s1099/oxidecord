//! Messages and everything hanging off one: attachments, replies, reactions.

use std::sync::Arc;

use twilight_model::id::{
    Id,
    marker::{EmojiMarker, MessageMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::embed::{Embed, convert_embed};
use super::markdown::{self, Inline, Markdown};
use super::time::format_timestamp;
use super::user::small_avatar_url;

#[derive(Clone)]
pub struct Message {
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub content: String,
    /// `content`, parsed. Shared, since a message is cloned more often than
    /// its text changes.
    pub markdown: Arc<Markdown>,
    /// The users the content mentions, which is how `<@id>` gets a name.
    pub mentions: Vec<MentionedUser>,
    /// Whether the message pings everyone who can see it: an `@everyone` or
    /// `@here` its author had permission to use. Discord only sets it when the
    /// ping actually went out, so it's what decides the highlight, not the text.
    pub mention_everyone: bool,
    /// The roles the message pings.
    pub mention_roles: Vec<Id<RoleMarker>>,
    pub timestamp: String,
    /// Whether the message has been edited since it was sent, which the view
    /// marks with "(edited)" after the content.
    pub edited: bool,
    pub images: Vec<ImageAttachment>,
    /// Video attachments, played inline by the platform decoder.
    pub videos: Vec<VideoAttachment>,
    /// Rich embeds under the content: link previews, bot cards, and so on.
    pub embeds: Vec<Embed>,
    /// The message this one is a reply to, when it references another. Carries
    /// just enough to render the quoted preview above the message.
    pub reply: Option<MessageReference>,
    /// Reactions on the message, in Discord's order (first reacted first).
    pub reactions: Vec<Reaction>,
}

impl Message {
    /// Takes on what an edit can change from `edited`, the same message as the
    /// server now has it.
    ///
    /// Only the edited parts are copied: an update doesn't reliably carry the
    /// author's guild member (so the nickname would fall back to the global
    /// name), the quoted reply, or the current user's own reactions, and none
    /// of those change with an edit anyway.
    pub fn apply_edit(&mut self, edited: Message) {
        self.content = edited.content;
        self.markdown = edited.markdown;
        self.mentions = edited.mentions;
        self.mention_everyone = edited.mention_everyone;
        self.mention_roles = edited.mention_roles;
        self.edited = edited.edited;
        // Attachments can be removed in an edit, and links added or removed
        // change the embeds.
        self.images = edited.images;
        self.videos = edited.videos;
        self.embeds = edited.embeds;
    }

    /// Replaces the text, as an edit made locally does before the server
    /// confirms it.
    pub fn set_content(&mut self, content: String) {
        self.markdown = Arc::new(Markdown::parse(&content));
        self.content = content;
    }
}

/// A user mentioned in a message, named the way the message's guild knows
/// them.
#[derive(Clone)]
pub struct MentionedUser {
    pub id: Id<UserMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
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
            Self::Custom { id, animated, .. } => Some(cdn::emoji_url(id, *animated)),
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
    /// The original's content parsed for the preview line, inline formatting
    /// only.
    pub preview: Vec<Inline>,
    pub mentions: Vec<MentionedUser>,
}

#[derive(Clone)]
pub struct ImageAttachment {
    pub url: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// inline preview while preserving aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Set when the attachment is marked as a spoiler.
    pub spoiler: Option<SpoilerCover>,
}

/// What an attachment marked as a spoiler shows until it's clicked open.
#[derive(Clone)]
pub struct SpoilerCover {
    /// The picture asked of the media proxy at a few pixels across. Drawn at
    /// full size it smears into a blur, which gpui has no filter for — its
    /// sampler filters linearly, so the upscale does the blurring.
    pub url: String,
}

impl SpoilerCover {
    /// The longest side of the picture a cover is fetched at, in pixels. Only
    /// the broad colours should survive being blown back up to preview size —
    /// at 16 a face or a line of text was still readable through the smear.
    pub const SIZE: u32 = 6;
}

/// A video attachment, played inline by [`crate::platform::video`].
#[derive(Clone)]
pub struct VideoAttachment {
    /// Discord's media-proxy URL, which is what gets decoded. Both this and
    /// `poster_url` carry a signed query string and expire, so neither is
    /// worth caching past the session.
    pub url: String,
    /// A still from the video, asked of the media proxy so the message can
    /// show something before anyone presses play. Discord doesn't promise one
    /// for every codec, so the poster is drawn over a themed card that stands
    /// on its own when the request comes back empty.
    pub poster_url: String,
    pub filename: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// card up front so the message doesn't reflow once playback starts.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: u64,
    /// Set when the attachment is marked as a spoiler. The cover is a blurred
    /// poster frame.
    pub spoiler: Option<SpoilerCover>,
}

pub(in crate::discord) fn convert_message(message: twilight_model::channel::Message) -> Message {
    let author_avatar_url = small_avatar_url(&message.author);
    // Prefer the author's per-guild nickname, then their global display name,
    // then their username. `member` is present on messages fetched from a
    // guild channel but not on ones we just sent, hence the fallbacks.
    let author_name = message
        .member
        .and_then(|member| member.nick)
        .or(message.author.global_name)
        .unwrap_or_else(|| message.author.name.clone());

    Message {
        id: message.id,
        author_id: message.author.id,
        author_name,
        author_avatar_url,
        markdown: Arc::new(Markdown::parse(&message.content)),
        mentions: convert_mentions(message.mentions),
        mention_everyone: message.mention_everyone,
        mention_roles: message.mention_roles,
        content: message.content,
        timestamp: format_timestamp(message.timestamp),
        edited: message.edited_timestamp.is_some(),
        images: message
            .attachments
            .iter()
            .filter(|attachment| is_image(attachment))
            .map(|attachment| ImageAttachment {
                url: cdn::scaled_url(
                    &attachment.proxy_url,
                    attachment.width,
                    attachment.height,
                    PREVIEW_MAX_WIDTH,
                    PREVIEW_MAX_HEIGHT,
                ),
                width: attachment.width.map(|w| w as u32),
                height: attachment.height.map(|h| h as u32),
                spoiler: is_spoiler(attachment).then(|| SpoilerCover {
                    url: cdn::scaled_url(
                        &attachment.proxy_url,
                        attachment.width,
                        attachment.height,
                        SpoilerCover::SIZE,
                        SpoilerCover::SIZE,
                    ),
                }),
            })
            .collect(),
        videos: message
            .attachments
            .iter()
            .filter(|attachment| is_video(attachment))
            .map(|attachment| VideoAttachment {
                url: attachment.proxy_url.clone(),
                poster_url: poster_url(attachment, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT),
                spoiler: is_spoiler(attachment).then(|| SpoilerCover {
                    url: poster_url(attachment, SpoilerCover::SIZE, SpoilerCover::SIZE),
                }),
                filename: attachment.filename.clone(),
                width: attachment.width.map(|w| w as u32),
                height: attachment.height.map(|h| h as u32),
                size: attachment.size,
            })
            .collect(),
        embeds: message.embeds.into_iter().map(convert_embed).collect(),
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

/// The mentioned users, by guild nickname where the message carries one.
/// twilight's mention model drops the global display name, so the username is
/// the fallback.
fn convert_mentions(
    mentions: Vec<twilight_model::channel::message::Mention>,
) -> Vec<MentionedUser> {
    mentions
        .into_iter()
        .map(|mention| MentionedUser {
            id: mention.id,
            avatar_url: mention
                .avatar
                .map(|hash| cdn::small_avatar_url(mention.id.get(), &hash.to_string())),
            name: mention
                .member
                .and_then(|member| member.nick)
                .unwrap_or(mention.name),
        })
        .collect()
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
    MessageReference {
        author_name: referenced
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| referenced.author.name.clone()),
        author_avatar_url: small_avatar_url(&referenced.author),
        content: single_line_preview(&referenced.content),
        preview: markdown::parse_preview(&referenced.content),
        mentions: convert_mentions(referenced.mentions),
    }
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

const PREVIEW_MAX_WIDTH: u32 = 480;
const PREVIEW_MAX_HEIGHT: u32 = 390;

/// Whether the uploader marked an attachment as a spoiler. There's no flag for
/// it in the API: the client prefixes the filename, and every client reads it
/// back the same way.
fn is_spoiler(attachment: &twilight_model::channel::Attachment) -> bool {
    attachment.filename.starts_with(SPOILER_PREFIX)
}

/// The filename prefix that marks an attachment as a spoiler.
pub const SPOILER_PREFIX: &str = "SPOILER_";

/// Whether an attachment is an image we can render inline. Prefers Discord's
/// reported media type and falls back to the filename extension.
fn is_image(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("image/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Whether an attachment is a video. Mirrors [`is_image`]: Discord's reported
/// media type first, filename extension second.
///
/// The extensions are the containers the platform decoders actually open, not
/// every `video/*` Discord will accept — an attachment listed here that turns
/// out to be undecodable still falls back to the poster card.
fn is_video(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("video/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".mp4", ".webm", ".mov", ".mkv", ".m4v", ".avi"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Asks the media proxy for a still frame of a video, fitted in the given box.
/// Discord serves one for the common codecs and 404s for the rest, which the
/// card is built to survive.
fn poster_url(attachment: &twilight_model::channel::Attachment, width: u32, height: u32) -> String {
    let separator = if attachment.proxy_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{}{separator}format=jpeg&width={width}&height={height}",
        attachment.proxy_url
    )
}
