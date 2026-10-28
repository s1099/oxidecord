//! Messages and everything hanging off one: attachments, replies, reactions.

use std::sync::Arc;

use serde::Deserialize;
use twilight_model::channel::AttachmentFlags;
use twilight_model::channel::message::{MessageReferenceType, MessageSnapshot, MessageType};
use twilight_model::id::{
    Id,
    marker::{EmojiMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::embed::{Embed, EmbedLayout, convert_embed};
use super::guild::GuildMember;
use super::markdown::{self, Inline, Markdown};
use super::system::{SystemEvent, convert_system};
use super::time::{
    format_local_day, format_local_full, format_message_clock, format_message_time, local_day,
};
use super::upload::Upload;
use super::user::{CurrentUser, small_avatar_url};

#[derive(Clone)]
pub struct Message {
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub author_bot: bool,
    pub content: String,
    /// `content`, parsed. Shared since messages are cloned more often than edited.
    pub markdown: Arc<Markdown>,
    pub mentions: Vec<MentionedUser>,
    /// Discord only sets it when an `@everyone`/`@here` actually pinged, so it,
    /// not the text, decides the highlight.
    pub mention_everyone: bool,
    pub mention_roles: Vec<Id<RoleMarker>>,
    /// Unix seconds.
    pub timestamp: i64,
    /// Unix seconds.
    pub edited: Option<i64>,
    pub images: Vec<ImageAttachment>,
    pub videos: Vec<VideoAttachment>,
    pub files: Vec<FileAttachment>,
    pub embeds: Vec<Embed>,
    pub reply: Option<Reply>,
    pub forward: Option<ForwardedMessage>,
    /// In Discord's order (first reacted first).
    pub reactions: Vec<Reaction>,
    /// Drawn as a one-line notice instead of the author's message.
    pub system: Option<SystemEvent>,
    pub delivery: Delivery,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// Shown optimistically; its id is the request's nonce so the confirmed
    /// copy can replace it. The files have no URL until then.
    Sending {
        upload: Option<Upload>,
    },
    /// Stays in the list so the text isn't lost.
    Failed,
}

impl Message {
    /// Whether the message's text is drawn; see [`is_bare_media_link`].
    pub fn shows_text(&self) -> bool {
        !self.content.is_empty() && !is_bare_media_link(&self.content, &self.embeds)
    }

    /// `id` is the request's nonce, a snowflake for the send time so it sorts
    /// like a real id. Mentions resolve once the confirmed copy arrives.
    pub fn pending(
        id: Id<MessageMarker>,
        author: &CurrentUser,
        content: String,
        timestamp: i64,
        reply_to: Option<&Message>,
        upload: Option<Upload>,
    ) -> Self {
        Self {
            id,
            author_id: author.id,
            author_name: author.name.clone(),
            author_avatar_url: author.avatar_url.clone(),
            author_bot: false,
            markdown: Arc::new(Markdown::parse(&content)),
            content,
            mentions: Vec::new(),
            mention_everyone: false,
            mention_roles: Vec::new(),
            timestamp,
            edited: None,
            images: Vec::new(),
            videos: Vec::new(),
            files: Vec::new(),
            embeds: Vec::new(),
            reply: reply_to.map(|message| Reply::Quote(MessageReference::quoting(message))),
            forward: None,
            reactions: Vec::new(),
            system: None,
            delivery: Delivery::Sending { upload },
        }
    }

    /// Copies only what an edit can change: an update doesn't reliably carry the
    /// author's guild member (the nickname would fall back to the global name),
    /// the quoted reply, or the user's own reactions.
    pub fn apply_edit(&mut self, edited: Message) {
        self.content = edited.content;
        self.markdown = edited.markdown;
        self.mentions = edited.mentions;
        self.mention_everyone = edited.mention_everyone;
        self.mention_roles = edited.mention_roles;
        self.edited = edited.edited;
        // Edits can remove attachments and add or remove embeds.
        self.images = edited.images;
        self.videos = edited.videos;
        self.files = edited.files;
        self.embeds = edited.embeds;
    }

    pub fn timestamp_label(&self) -> String {
        format_message_time(self.timestamp)
    }

    /// Time of day alone, for a continuation message's gutter.
    pub fn clock_label(&self) -> String {
        format_message_clock(self.timestamp)
    }

    /// Whether `previous` was sent on an earlier local day, which a divider marks.
    pub fn starts_day_after(&self, previous: &Message) -> bool {
        local_day(self.timestamp) != local_day(previous.timestamp)
    }

    /// The divider's `October 27, 2026`.
    pub fn day_label(&self) -> String {
        format_local_day(self.timestamp)
    }

    /// Long form for the "(edited)" tooltip.
    pub fn edited_label(&self) -> Option<String> {
        self.edited.map(format_local_full)
    }

    /// Turns a quote of any of `deleted` into the deleted-original line, as
    /// Discord's client does. Returns whether it changed.
    pub fn forget_deleted_reply(&mut self, deleted: &[Id<MessageMarker>]) -> bool {
        match &self.reply {
            Some(Reply::Quote(quote)) if deleted.contains(&quote.id) => {
                self.reply = Some(Reply::Deleted);
                true
            }
            _ => false,
        }
    }

    /// For a local edit shown before the server confirms it.
    pub fn set_content(&mut self, content: String) {
        self.markdown = Arc::new(Markdown::parse(&content));
        self.content = content;
    }
}

#[derive(Clone)]
pub struct MentionedUser {
    pub id: Id<UserMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    pub count: u64,
    pub me: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
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
    /// `None` for unicode emoji, which render as text.
    pub fn image_url(&self) -> Option<String> {
        match self {
            Self::Unicode(_) => None,
            Self::Custom { id, animated, .. } => Some(cdn::emoji_url(id, *animated)),
        }
    }

    /// The `:name:` used in "reacted with" lines; a unicode emoji without a
    /// shortcode is its own label.
    pub fn label(&self) -> String {
        match self {
            Self::Unicode(text) => emojis::get(text)
                .and_then(|emoji| emoji.shortcode())
                .map_or_else(|| text.clone(), |code| format!(":{code}:")),
            Self::Custom { name, .. } if name.is_empty() => ":deleted emoji:".into(),
            Self::Custom { name, .. } => format!(":{name}:"),
        }
    }
}

#[derive(Clone)]
pub enum Reply {
    Quote(MessageReference),
    /// The original is gone; Discord sends the reply without it.
    Deleted,
}

/// The quoted preview above a reply.
#[derive(Clone)]
pub struct MessageReference {
    /// Clicking the quote jumps to it.
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    /// Empty when the original had no text.
    pub content: String,
    /// Inline formatting only.
    pub preview: Vec<Inline>,
    pub mentions: Vec<MentionedUser>,
}

impl MessageReference {
    fn quoting(message: &Message) -> Self {
        let content = match &message.forward {
            Some(forward) if message.content.is_empty() => forward.content.as_str(),
            _ => message.content.as_str(),
        };
        Self {
            id: message.id,
            author_id: message.author_id,
            author_name: message.author_name.clone(),
            author_avatar_url: message.author_avatar_url.clone(),
            content: single_line_preview(content),
            preview: markdown::parse_preview(content),
            mentions: message.mentions.clone(),
        }
    }
}

/// A snapshot taken at forward time. Discord sends it without an author.
#[derive(Clone)]
pub struct ForwardedMessage {
    pub content: String,
    pub markdown: Arc<Markdown>,
    pub mentions: Vec<MentionedUser>,
    /// Unix seconds.
    pub timestamp: i64,
    pub edited: Option<i64>,
    pub images: Vec<ImageAttachment>,
    pub videos: Vec<VideoAttachment>,
    pub files: Vec<FileAttachment>,
    pub embeds: Vec<Embed>,
    /// `None` for a direct message.
    pub guild_id: Option<Id<GuildMarker>>,
}

impl ForwardedMessage {
    /// Whether the message's text is drawn; see [`is_bare_media_link`].
    pub fn shows_text(&self) -> bool {
        !self.content.is_empty() && !is_bare_media_link(&self.content, &self.embeds)
    }

    pub fn timestamp_label(&self) -> String {
        format_message_time(self.timestamp)
    }

    pub fn edited_label(&self) -> Option<String> {
        self.edited.map(format_local_full)
    }
}

#[derive(Clone)]
pub struct ImageAttachment {
    pub url: String,
    /// The proxy's unscaled original, for the image viewer and the browser.
    pub source: String,
    /// Intrinsic pixels, to size the preview with its aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether [`Self::source`] must be requested as animated WebP to stay
    /// moving at another size; see [`cdn::wants_animated_webp`].
    pub animated_webp: bool,
    pub spoiler: Option<SpoilerCover>,
}

#[derive(Clone)]
pub struct SpoilerCover {
    /// The picture at a few pixels across; upscaled it smears into a blur,
    /// which gpui has no filter for (its sampler filters linearly).
    pub url: String,
}

impl SpoilerCover {
    /// Longest side in pixels. At 16 a face or text was still readable through
    /// the smear.
    pub const SIZE: u32 = 6;
}

#[derive(Clone)]
pub struct FileAttachment {
    /// The CDN URL; the media proxy only serves images and video.
    pub url: String,
    pub filename: String,
    pub size: u64,
}

#[derive(Clone)]
pub struct VideoAttachment {
    /// Media-proxy URL, which is what gets decoded. This and `poster_url` are
    /// signed and expire, so don't cache them past the session.
    pub url: String,
    /// Discord doesn't serve a still for every codec, so the poster is drawn
    /// over a themed card that stands alone when the request comes back empty.
    pub poster_url: String,
    pub filename: String,
    /// Intrinsic pixels, to size the card before playback without reflow.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: u64,
    /// A blurred poster frame.
    pub spoiler: Option<SpoilerCover>,
}

/// `GET /channels/{id}/messages/pins`, most recently pinned first.
#[derive(Deserialize)]
pub(in crate::discord) struct RawPins {
    pub items: Vec<RawPin>,
    pub has_more: bool,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawPin {
    /// ISO 8601; the next page's `before` takes it.
    pub pinned_at: String,
    pub message: twilight_model::channel::Message,
}

/// Only gateway dispatches carry a member; fetched history doesn't, and
/// webhooks aren't members.
pub(in crate::discord) fn author_member(
    message: &twilight_model::channel::Message,
) -> Option<GuildMember> {
    let member = message.member.as_ref()?;
    message.guild_id?;
    let author = &message.author;
    let name = [member.nick.as_ref(), author.global_name.as_ref()]
        .into_iter()
        .flatten()
        .find(|name| !name.is_empty())
        .unwrap_or(&author.name);
    Some(GuildMember {
        user_id: author.id,
        name: name.clone(),
        username: Some(author.name.clone()).filter(|name| !name.is_empty()),
        avatar_url: small_avatar_url(author),
    })
}

pub(in crate::discord) fn convert_message(message: twilight_model::channel::Message) -> Message {
    let author_avatar_url = small_avatar_url(&message.author);
    let system = convert_system(&message);
    // Nickname, then global name, then username. Only gateway dispatches carry
    // `member`; fetched history and send responses don't, so the view names
    // authors by the guild's members where it can.
    let author_name = message
        .member
        .and_then(|member| member.nick)
        .filter(|nick| !nick.is_empty())
        .or(message.author.global_name)
        .unwrap_or_else(|| message.author.name.clone());
    let (images, videos, files) = convert_attachments(&message.attachments);
    // A forward uses the same reference as a reply, so check its type.
    let is_forward = message
        .reference
        .as_ref()
        .is_some_and(|reference| reference.kind == MessageReferenceType::Forward);
    let forward = is_forward
        .then(|| message.message_snapshots.into_iter().next())
        .flatten()
        .map(convert_snapshot);

    Message {
        id: message.id,
        author_id: message.author.id,
        author_name,
        author_avatar_url,
        author_bot: message.author.bot,
        markdown: Arc::new(Markdown::parse(&message.content)),
        mentions: convert_mentions(message.mentions),
        mention_everyone: message.mention_everyone,
        mention_roles: message.mention_roles,
        content: message.content,
        timestamp: message.timestamp.as_secs(),
        edited: message.edited_timestamp.map(|edited| edited.as_secs()),
        images,
        videos,
        files,
        embeds: message.embeds.into_iter().map(convert_embed).collect(),
        reply: match message.referenced_message {
            _ if is_forward => None,
            Some(referenced) => Some(Reply::Quote(convert_reference(*referenced))),
            None if message.kind == MessageType::Reply => Some(Reply::Deleted),
            None => None,
        },
        forward,
        reactions: message
            .reactions
            .into_iter()
            .map(|reaction| Reaction {
                emoji: convert_reaction_emoji(reaction.emoji),
                count: reaction.count,
                me: reaction.me,
            })
            .collect(),
        system,
        delivery: Delivery::Sent,
    }
}

fn convert_attachments(
    attachments: &[twilight_model::channel::Attachment],
) -> (
    Vec<ImageAttachment>,
    Vec<VideoAttachment>,
    Vec<FileAttachment>,
) {
    let images = attachments
        .iter()
        .filter(|attachment| is_image(attachment))
        .map(|attachment| {
            let animated = attachment
                .flags
                .is_some_and(|flags| flags.contains(AttachmentFlags::IS_ANIMATED));
            let animated_webp =
                cdn::wants_animated_webp(animated, attachment.content_type.as_deref());
            let url = cdn::scaled_url(
                &attachment.proxy_url,
                attachment.width,
                attachment.height,
                PREVIEW_MAX_WIDTH,
                PREVIEW_MAX_HEIGHT,
            );
            ImageAttachment {
                url: if animated_webp {
                    cdn::animated_webp(url)
                } else {
                    url
                },
                source: attachment.proxy_url.clone(),
                width: attachment.width.map(|w| w as u32),
                height: attachment.height.map(|h| h as u32),
                animated_webp,
                spoiler: is_spoiler(attachment).then(|| SpoilerCover {
                    url: cdn::scaled_url(
                        &attachment.proxy_url,
                        attachment.width,
                        attachment.height,
                        SpoilerCover::SIZE,
                        SpoilerCover::SIZE,
                    ),
                }),
            }
        })
        .collect();
    let videos = attachments
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
        .collect();
    let files = attachments
        .iter()
        .filter(|attachment| !is_image(attachment) && !is_video(attachment))
        .map(|attachment| FileAttachment {
            url: attachment.url.clone(),
            filename: attachment.filename.clone(),
            size: attachment.size,
        })
        .collect();
    (images, videos, files)
}

fn convert_snapshot(snapshot: MessageSnapshot) -> ForwardedMessage {
    let message = snapshot.message;
    let (images, videos, files) = convert_attachments(&message.attachments);
    ForwardedMessage {
        markdown: Arc::new(Markdown::parse(&message.content)),
        content: message.content,
        mentions: convert_mentions(message.mentions),
        timestamp: message.timestamp.as_secs(),
        edited: message.edited_timestamp.map(|edited| edited.as_secs()),
        images,
        videos,
        files,
        embeds: message.embeds.into_iter().map(convert_embed).collect(),
        guild_id: snapshot.guild_id,
    }
}

/// twilight's mention model drops the global display name, so the username is
/// the fallback after the nickname.
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

pub(in crate::discord) fn convert_reaction_emoji(
    emoji: twilight_model::channel::message::EmojiReactionType,
) -> ReactionEmoji {
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

/// Referenced messages carry no guild `member`, so the name is the global name,
/// else the username.
fn convert_reference(referenced: twilight_model::channel::Message) -> MessageReference {
    // A forward has no text of its own; quote what it carries.
    let content = match referenced.message_snapshots.first() {
        Some(snapshot) if referenced.content.is_empty() => snapshot.message.content.as_str(),
        _ => referenced.content.as_str(),
    };
    MessageReference {
        id: referenced.id,
        author_id: referenced.author.id,
        author_name: referenced
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| referenced.author.name.clone()),
        author_avatar_url: small_avatar_url(&referenced.author),
        content: single_line_preview(content),
        preview: markdown::parse_preview(content),
        mentions: convert_mentions(referenced.mentions),
    }
}

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

/// Whether a message is nothing but a link to the image or GIF its embed
/// draws. Discord hides the link then; any other text keeps it.
fn is_bare_media_link(content: &str, embeds: &[Embed]) -> bool {
    let content = content.trim();
    !content.contains(char::is_whitespace)
        && embeds.iter().any(|embed| {
            embed.layout == EmbedLayout::Media
                && embed
                    .url
                    .as_deref()
                    .is_some_and(|url| url.trim_end_matches('/') == content.trim_end_matches('/'))
        })
}

/// The API has no spoiler flag; clients prefix the filename.
fn is_spoiler(attachment: &twilight_model::channel::Attachment) -> bool {
    attachment.filename.starts_with(SPOILER_PREFIX)
}

pub const SPOILER_PREFIX: &str = "SPOILER_";

/// Discord's reported media type first, filename extension second.
fn is_image(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("image/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Like [`is_image`]. The extensions are the containers the platform decoders
/// open; an undecodable one still falls back to the poster card.
fn is_video(attachment: &twilight_model::channel::Attachment) -> bool {
    if let Some(content_type) = &attachment.content_type {
        return content_type.starts_with("video/");
    }
    let name = attachment.filename.to_ascii_lowercase();
    [".mp4", ".webm", ".mov", ".mkv", ".m4v", ".avi"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Discord 404s for uncommon codecs, which the card is built to survive.
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
