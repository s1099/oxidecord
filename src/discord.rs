use std::sync::OnceLock;

use serde::Deserialize;
use tokio::runtime::Handle;
use twilight_http::request::Request;
use twilight_http::response::marker::ListBody;
use twilight_http::routing::Route;
use twilight_http::Client as HttpClient;
use twilight_model::channel::ChannelType;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker},
    Id,
};
use twilight_model::util::Timestamp;

static RUNTIME: OnceLock<Handle> = OnceLock::new();

/// Returns a handle to a lazily-started background Tokio runtime.
///
/// twilight-http needs to run inside a Tokio context, but gpui has its own
/// executor, so we keep a dedicated runtime alive on its own thread.
pub fn runtime_handle() -> &'static Handle {
    RUNTIME.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            tx.send(rt.handle().clone())
                .expect("failed to send tokio runtime handle");
            rt.block_on(std::future::pending::<()>());
        });
        rx.recv().expect("failed to receive tokio runtime handle")
    })
}

#[derive(Deserialize)]
struct AuthFile {
    #[serde(rename = "userToken")]
    user_token: String,
}

/// Reads the user's Discord token from `auth.json` in the working directory.
pub fn load_token() -> Option<String> {
    let content = std::fs::read_to_string("auth.json").ok()?;
    serde_json::from_str::<AuthFile>(&content)
        .ok()
        .map(|auth| auth.user_token)
}

#[derive(Clone)]
pub struct Guild {
    pub id: Id<GuildMarker>,
    pub name: String,
    pub icon_url: Option<String>,
}

/// Fetches the current user's guilds and invokes `on_done` with the result.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn fetch_guilds(
    token: String,
    on_done: impl FnOnce(Result<Vec<Guild>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .current_user_guilds()
                .await
                .map_err(|err| err.to_string())?;
            let guilds = response.models().await.map_err(|err| err.to_string())?;

            Ok(guilds
                .into_iter()
                .map(|guild| {
                    let icon_url = guild.icon.map(|hash| {
                        format!("https://cdn.discordapp.com/icons/{}/{}.webp?size=100&quality=lossless", guild.id, hash)
                    });
                    Guild {
                        id: guild.id,
                        name: guild.name,
                        icon_url,
                    }
                })
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
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

/// Fetches a guild's channels and invokes `on_done` with the result.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread. Channel types the UI
/// can't display (threads, directories, ...) are filtered out.
pub fn fetch_channels(
    token: String,
    guild_id: Id<GuildMarker>,
    on_done: impl FnOnce(Result<Vec<Channel>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .guild_channels(guild_id)
                .await
                .map_err(|err| err.to_string())?;
            let channels = response.models().await.map_err(|err| err.to_string())?;

            Ok(channels
                .into_iter()
                .filter_map(|channel| {
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
                })
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// A 1:1 or group direct-message conversation from the user's DM list.
#[derive(Clone)]
pub struct DirectMessage {
    pub id: Id<ChannelMarker>,
    /// Display label: the other user for a DM, the group's name (or its
    /// members' names) for a group DM.
    pub name: String,
    pub avatar_url: Option<String>,
    /// Snowflake of the last message, used only to order conversations by
    /// recency like Discord does. `None` (no messages yet) sorts last.
    last_message_id: Option<u64>,
}

fn convert_dm(channel: twilight_model::channel::Channel) -> Option<DirectMessage> {
    let last_message_id = channel.last_message_id.map(|id| id.get());
    match channel.kind {
        // 1:1 DM: a single recipient, the other user.
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
        // Group DM: a custom icon and name, each optional. Fall back to the
        // joined recipient names when the group is unnamed, like Discord.
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
                        .map(|user| user.global_name.clone().unwrap_or_else(|| user.name.clone()))
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

/// Fetches the current user's open DM and group-DM conversations, ordered
/// most-recently-active first, and invokes `on_done` with the result.
///
/// twilight has no typed helper for `GET /users/@me/channels`, so this issues
/// the route through the client's low-level request path. Runs on the
/// background Tokio runtime; `on_done` is called from that runtime's thread.
pub fn fetch_dms(
    token: String,
    on_done: impl FnOnce(Result<Vec<DirectMessage>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let request = Request::from_route(&Route::GetUserPrivateChannels);
            let response = client
                .request::<ListBody<twilight_model::channel::Channel>>(request)
                .await
                .map_err(|err| err.to_string())?;
            let channels = response.models().await.map_err(|err| err.to_string())?;

            let mut dms: Vec<DirectMessage> =
                channels.into_iter().filter_map(convert_dm).collect();
            dms.sort_by_key(|dm| std::cmp::Reverse(dm.last_message_id));
            Ok(dms)
        }
        .await;

        on_done(result);
    });
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
}

/// An image attachment on a message, ready to display inline.
#[derive(Clone)]
pub struct ImageAttachment {
    pub url: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to size the
    /// inline preview while preserving aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
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
    let separator = if attachment.proxy_url.contains('?') { '&' } else { '?' };
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

/// Formats a Discord timestamp as `YYYY-MM-DD HH:MM` (UTC).
fn format_timestamp(timestamp: Timestamp) -> String {
    // ISO 8601 form is `2021-08-10T11:16:37.020000+00:00`.
    let iso = timestamp.iso_8601().to_string();
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => iso,
    }
}

fn convert_message(message: twilight_model::channel::Message) -> Message {
    let author_avatar_url = message.author.avatar.map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{}/{}.webp?size=80",
            message.author.id, hash
        )
    });
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
    }
}

/// How many messages one `fetch_messages` call requests; a response with
/// fewer means the start of the channel's history was reached.
pub const MESSAGE_PAGE_SIZE: usize = 50;

/// Fetches a page of messages in a channel (oldest first) and invokes
/// `on_done` with the result. With `before`, fetches the page of messages
/// older than that message; otherwise fetches the most recent page.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn fetch_messages(
    token: String,
    channel_id: Id<ChannelMarker>,
    before: Option<Id<MessageMarker>>,
    on_done: impl FnOnce(Result<Vec<Message>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let request = client
                .channel_messages(channel_id)
                .limit(MESSAGE_PAGE_SIZE as u16);
            // `.before()` changes the request's type, so await per branch.
            let response = match before {
                Some(before) => request.before(before).await,
                None => request.await,
            }
            .map_err(|err| err.to_string())?;
            let mut messages = response.models().await.map_err(|err| err.to_string())?;

            // The API returns newest first; the UI renders oldest first.
            messages.reverse();
            Ok(messages.into_iter().map(convert_message).collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// Sends a message to a channel and invokes `on_done` with the created
/// message.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn send_message(
    token: String,
    channel_id: Id<ChannelMarker>,
    content: String,
    on_done: impl FnOnce(Result<Message, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .create_message(channel_id)
                .content(&content)
                .await
                .map_err(|err| err.to_string())?;
            let message = response.model().await.map_err(|err| err.to_string())?;

            Ok(convert_message(message))
        }
        .await;

        on_done(result);
    });
}
