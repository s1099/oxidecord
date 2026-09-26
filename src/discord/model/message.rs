//! Messages and everything hanging off one: attachments, replies, reactions.

use twilight_model::id::{
    Id,
    marker::{EmojiMarker, MessageMarker, UserMarker},
};

use super::cdn;
use super::embed::{Embed, convert_embed};
use super::time::format_timestamp;
use super::user::small_avatar_url;

#[derive(Clone)]
pub struct Message {
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub content: String,
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
        self.edited = edited.edited;
        // Attachments can be removed in an edit, and links added or removed
        // change the embeds.
        self.images = edited.images;
        self.videos = edited.videos;
        self.embeds = edited.embeds;
    }
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
}

#[derive(Clone)]
pub struct ImageAttachment {
    pub url: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// inline preview while preserving aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
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
            })
            .collect(),
        videos: message
            .attachments
            .iter()
            .filter(|attachment| is_video(attachment))
            .map(|attachment| VideoAttachment {
                url: attachment.proxy_url.clone(),
                poster_url: poster_url(attachment),
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

/// Asks the media proxy for a still frame of a video, at the size the poster
/// card renders it. Discord serves one for the common codecs and 404s for the
/// rest, which the card is built to survive.
fn poster_url(attachment: &twilight_model::channel::Attachment) -> String {
    let separator = if attachment.proxy_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{}{separator}format=jpeg&width={PREVIEW_MAX_WIDTH}&height={PREVIEW_MAX_HEIGHT}",
        attachment.proxy_url
    )
}
