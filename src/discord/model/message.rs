//! Messages and everything hanging off one: attachments, replies, reactions.

use twilight_model::id::{
    Id,
    marker::{EmojiMarker, MessageMarker, UserMarker},
};

use super::cdn;
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
        images: message
            .attachments
            .iter()
            .filter(|attachment| is_image(attachment))
            .map(|attachment| ImageAttachment {
                url: preview_image_url(attachment),
                width: attachment.width.map(|w| w as u32),
                height: attachment.height.map(|h| h as u32),
            })
            .collect(),
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

/// Asks the CDN for the image already scaled to the size the message list
/// renders it at, so the full-resolution original is never downloaded.
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
fn is_image(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("image/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}
