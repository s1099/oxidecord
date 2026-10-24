//! Messages and everything hanging off one: attachments, replies, reactions.

use std::sync::Arc;

use serde::Deserialize;
use twilight_model::channel::AttachmentFlags;
use twilight_model::channel::message::{MessageReferenceType, MessageSnapshot};
use twilight_model::id::{
    Id,
    marker::{EmojiMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::embed::{Embed, EmbedLayout, convert_embed};
use super::guild::GuildMember;
use super::markdown::{self, Inline, Markdown};
use super::time::{format_local_full, format_message_time};
use super::user::{CurrentUser, small_avatar_url};

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
    pub mention_roles: Vec<Id<RoleMarker>>,
    /// When the message was sent, in Unix seconds.
    pub timestamp: i64,
    /// When the message was last edited, in Unix seconds, which the view
    /// marks with "(edited)" after the content.
    pub edited: Option<i64>,
    pub images: Vec<ImageAttachment>,
    pub videos: Vec<VideoAttachment>,
    pub embeds: Vec<Embed>,
    /// The message this one is a reply to, when it references another. Carries
    /// just enough to render the quoted preview above the message.
    pub reply: Option<MessageReference>,
    /// The message this one forwards, as it stood when it was forwarded.
    pub forward: Option<ForwardedMessage>,
    /// Reactions on the message, in Discord's order (first reacted first).
    pub reactions: Vec<Reaction>,
    /// Whether the server has the message yet. Only ever not [`Delivery::Sent`]
    /// for one the user just sent, shown before Discord confirms it.
    pub delivery: Delivery,
}

/// How far a message sent from here has got.
#[derive(Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// Shown optimistically while the request is in flight. Its id is the
    /// nonce the request carried, so the confirmed copy can take its place.
    /// Carries the names of the files being uploaded with it, which have no
    /// URL to draw from yet.
    Sending {
        uploads: Vec<String>,
    },
    /// The request failed; the message stays in the list so the text isn't
    /// lost, like Discord's red failed message.
    Failed,
}

impl Message {
    /// Whether the message's text is drawn; see [`is_bare_media_link`].
    pub fn shows_text(&self) -> bool {
        !self.content.is_empty() && !is_bare_media_link(&self.content, &self.embeds)
    }

    /// A message the user is sending, as it's shown before the server has it.
    ///
    /// `id` is the request's nonce: a snowflake for the moment it was sent, so
    /// it sorts and dates like a real id. Mentions resolve once the confirmed
    /// copy arrives with the mentioned users attached.
    pub fn pending(
        id: Id<MessageMarker>,
        author: &CurrentUser,
        content: String,
        timestamp: i64,
        reply_to: Option<&Message>,
        uploads: Vec<String>,
    ) -> Self {
        Self {
            id,
            author_id: author.id,
            author_name: author.name.clone(),
            author_avatar_url: author.avatar_url.clone(),
            markdown: Arc::new(Markdown::parse(&content)),
            content,
            mentions: Vec::new(),
            mention_everyone: false,
            mention_roles: Vec::new(),
            timestamp,
            edited: None,
            images: Vec::new(),
            videos: Vec::new(),
            embeds: Vec::new(),
            reply: reply_to.map(MessageReference::quoting),
            forward: None,
            reactions: Vec::new(),
            delivery: Delivery::Sending { uploads },
        }
    }

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

    /// When the message was sent, as its header shows it: just the time for
    /// today, the short date and time before that.
    pub fn timestamp_label(&self) -> String {
        format_message_time(self.timestamp)
    }

    /// When the message was last edited, in the long form the "(edited)"
    /// marker's tooltip shows.
    pub fn edited_label(&self) -> Option<String> {
        self.edited.map(format_local_full)
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

#[derive(Clone)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    pub count: u64,
    /// Whether the current user is one of the reactors.
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
    /// CDN URL for a custom emote's image; `None` for unicode emoji, which are
    /// rendered as text.
    pub fn image_url(&self) -> Option<String> {
        match self {
            Self::Unicode(_) => None,
            Self::Custom { id, animated, .. } => Some(cdn::emoji_url(id, *animated)),
        }
    }

    /// The `:name:` Discord names the emoji by in "reacted with" lines. A
    /// unicode emoji without a known shortcode is named by itself.
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

/// A compact snapshot of the message a reply points at, used to render the
/// quoted preview line atop the reply.
#[derive(Clone)]
pub struct MessageReference {
    pub author_id: Id<UserMarker>,
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

impl MessageReference {
    /// The quote for a reply to `message`, built from the copy already loaded.
    fn quoting(message: &Message) -> Self {
        let content = match &message.forward {
            Some(forward) if message.content.is_empty() => forward.content.as_str(),
            _ => message.content.as_str(),
        };
        Self {
            author_id: message.author_id,
            author_name: message.author_name.clone(),
            author_avatar_url: message.author_avatar_url.clone(),
            content: single_line_preview(content),
            preview: markdown::parse_preview(content),
            mentions: message.mentions.clone(),
        }
    }
}

/// A copy of a forwarded message, taken when it was forwarded. Discord sends
/// it without an author, so the view credits where it came from instead.
#[derive(Clone)]
pub struct ForwardedMessage {
    pub content: String,
    pub markdown: Arc<Markdown>,
    pub mentions: Vec<MentionedUser>,
    /// When the original was sent, in Unix seconds.
    pub timestamp: i64,
    pub edited: Option<i64>,
    pub images: Vec<ImageAttachment>,
    pub videos: Vec<VideoAttachment>,
    pub embeds: Vec<Embed>,
    /// The guild the original was sent in; `None` for a direct message.
    pub guild_id: Option<Id<GuildMarker>>,
}

impl ForwardedMessage {
    /// Whether the message's text is drawn; see [`is_bare_media_link`].
    pub fn shows_text(&self) -> bool {
        !self.content.is_empty() && !is_bare_media_link(&self.content, &self.embeds)
    }

    /// When the original was sent, formatted like a message header's time.
    pub fn timestamp_label(&self) -> String {
        format_message_time(self.timestamp)
    }

    pub fn edited_label(&self) -> Option<String> {
        self.edited.map(format_local_full)
    }
}

#[derive(Clone)]
pub struct ImageAttachment {
    /// Scaled down to the inline preview's box.
    pub url: String,
    /// The proxy's unscaled original, which the image viewer asks for at the
    /// size of the window and opens in the browser.
    pub source: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// inline preview while preserving aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether [`Self::source`] has to be asked of the proxy as animated WebP
    /// to stay moving at another size; see [`cdn::wants_animated_webp`].
    pub animated_webp: bool,
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
    /// The cover is a blurred poster frame.
    pub spoiler: Option<SpoilerCover>,
}

/// A page of `GET /channels/{id}/messages/pins`, most recently pinned first.
#[derive(Deserialize)]
pub(in crate::discord) struct RawPins {
    pub items: Vec<RawPin>,
    pub has_more: bool,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawPin {
    /// ISO 8601, which is what the next page's `before` takes.
    pub pinned_at: String,
    pub message: twilight_model::channel::Message,
}

/// Who a guild message's author is there, from the member Discord sends along
/// with it. Only gateway dispatches carry one: fetched history doesn't, and
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
    // Prefer the author's per-guild nickname, then their global display name,
    // then their username. Only gateway dispatches carry `member`: fetched
    // history and the response to a send don't, so the view names authors by
    // the guild's members where it knows them.
    let author_name = message
        .member
        .and_then(|member| member.nick)
        .filter(|nick| !nick.is_empty())
        .or(message.author.global_name)
        .unwrap_or_else(|| message.author.name.clone());
    let (images, videos) = convert_attachments(&message.attachments);
    // A forward points at its original through the same reference a reply
    // does, so only a plain reference counts as a reply.
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
        markdown: Arc::new(Markdown::parse(&message.content)),
        mentions: convert_mentions(message.mentions),
        mention_everyone: message.mention_everyone,
        mention_roles: message.mention_roles,
        content: message.content,
        timestamp: message.timestamp.as_secs(),
        edited: message.edited_timestamp.map(|edited| edited.as_secs()),
        images,
        videos,
        embeds: message.embeds.into_iter().map(convert_embed).collect(),
        reply: message
            .referenced_message
            .filter(|_| !is_forward)
            .map(|referenced| convert_reference(*referenced)),
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
        delivery: Delivery::Sent,
    }
}

/// Splits attachments into the images and videos the view draws inline;
/// anything else isn't shown.
fn convert_attachments(
    attachments: &[twilight_model::channel::Attachment],
) -> (Vec<ImageAttachment>, Vec<VideoAttachment>) {
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
    (images, videos)
}

fn convert_snapshot(snapshot: MessageSnapshot) -> ForwardedMessage {
    let message = snapshot.message;
    let (images, videos) = convert_attachments(&message.attachments);
    ForwardedMessage {
        markdown: Arc::new(Markdown::parse(&message.content)),
        content: message.content,
        mentions: convert_mentions(message.mentions),
        timestamp: message.timestamp.as_secs(),
        edited: message.edited_timestamp.map(|edited| edited.as_secs()),
        images,
        videos,
        embeds: message.embeds.into_iter().map(convert_embed).collect(),
        guild_id: snapshot.guild_id,
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

/// Extracts the referenced message into a compact preview. Referenced messages
/// don't carry guild `member` data, so the author name falls back to the global
/// display name and then the username.
fn convert_reference(referenced: twilight_model::channel::Message) -> MessageReference {
    // A forward has no text of its own, so a reply to one quotes what it
    // carries.
    let content = match referenced.message_snapshots.first() {
        Some(snapshot) if referenced.content.is_empty() => snapshot.message.content.as_str(),
        _ => referenced.content.as_str(),
    };
    MessageReference {
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
/// Whether a message is nothing but a link to the image or GIF its embed
/// draws. Discord leaves the link out then, so a Tenor link posted on its own
/// reads as the GIF alone; any other text keeps it, link and all.
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

fn is_spoiler(attachment: &twilight_model::channel::Attachment) -> bool {
    attachment.filename.starts_with(SPOILER_PREFIX)
}

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
