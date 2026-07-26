use std::sync::{OnceLock, RwLock};

use tokio::runtime::Handle;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;
use twilight_http::request::Request;
use twilight_http::response::marker::ListBody;
use twilight_http::routing::Route;
use twilight_model::channel::ChannelType;
use twilight_model::gateway::payload::incoming::MessageCreate;
pub use twilight_model::guild::Permissions;
use twilight_model::http::attachment::Attachment;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};
use twilight_model::util::Timestamp;
use twilight_util::permission_calculator::PermissionCalculator;

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

const KEYRING_SERVICE: &str = "oxidecord";

const KEYRING_USER: &str = "discord-token";

/// In-process cache so the hot paths don't hit the OS credential store on
/// every request. `None` means not loaded yet.
static TOKEN_CACHE: RwLock<Option<Option<String>>> = RwLock::new(None);

fn keyring_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
}

pub fn load_token() -> Option<String> {
    if let Some(cached) = TOKEN_CACHE.read().ok()?.as_ref() {
        return cached.clone();
    }

    let token = match keyring_entry().and_then(|entry| entry.get_password()) {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            eprintln!("failed to read token from the credential store: {err}");
            None
        }
    };

    if let Ok(mut cache) = TOKEN_CACHE.write() {
        *cache = Some(token.clone());
    }
    token
}

/// Stores the user's Discord token in the OS credential store, replacing token already there.
pub fn save_token(token: &str) -> keyring::Result<()> {
    keyring_entry()?.set_password(token)?;
    if let Ok(mut cache) = TOKEN_CACHE.write() {
        *cache = Some(Some(token.to_owned()));
    }
    Ok(())
}

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

/// Fetches the signed-in user (`GET /users/@me`) and invokes `on_done` with
/// the result.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn fetch_current_user(
    token: String,
    on_done: impl FnOnce(Result<CurrentUser, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client.current_user().await.map_err(|err| err.to_string())?;
            let user = response.model().await.map_err(|err| err.to_string())?;
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
            Ok(CurrentUser {
                name,
                username: user.name,
                avatar_url,
            })
        }
        .await;

        on_done(result);
    });
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
/// Only channels the current user can view (holds the `VIEW_CHANNEL`
/// permission on) are returned; the rest are the channels Discord itself
/// hides from the sidebar. `base_permissions` and `owner` come from the guild
/// list (see [`Guild`]) and are the user's guild-wide permissions, so the only
/// per-guild extra request here is the member object — which of their roles
/// apply — used to resolve each channel's overwrites locally.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread. Channel types the UI
/// can't display (threads, directories, ...) are filtered out.
pub fn fetch_channels(
    token: String,
    guild_id: Id<GuildMarker>,
    base_permissions: Permissions,
    owner: bool,
    on_done: impl FnOnce(Result<Vec<Channel>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);

            // The member object tells us which roles the user has, so
            // role-specific channel overwrites can be matched.
            let member = client
                .current_user_guild_member(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .model()
                .await
                .map_err(|err| err.to_string())?;
            let channels = client
                .guild_channels(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .models()
                .await
                .map_err(|err| err.to_string())?;

            let user_id = member.user.id;
            // We already know the user's aggregate guild-wide permissions, so
            // seed the calculator with those as the `@everyone` baseline and
            // list the member's roles with empty permissions — the roles are
            // only needed by id, to match channel overwrites, not to recompute
            // the baseline.
            let member_roles: Vec<(Id<RoleMarker>, Permissions)> = member
                .roles
                .iter()
                .filter(|id| id.get() != guild_id.get())
                .map(|id| (*id, Permissions::empty()))
                .collect();

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

                    // Apply the channel's own overwrites to the baseline and
                    // drop the channel if the user can't even view it.
                    let overwrites = channel.permission_overwrites.as_deref().unwrap_or(&[]);
                    let mut calculator = PermissionCalculator::new(
                        guild_id,
                        user_id,
                        base_permissions,
                        &member_roles,
                    );
                    if owner {
                        calculator = calculator.owner_id(user_id);
                    }
                    let permissions = calculator.in_channel(channel.kind, overwrites);
                    if !permissions.contains(Permissions::VIEW_CHANNEL) {
                        return None;
                    }

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
    pub name: String,
    pub avatar_url: Option<String>,
    /// Snowflake of the last message, used only to order conversations by
    /// recency like Discord does. `None` (no messages yet) sorts last.
    last_message_id: Option<u64>,
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

            let mut dms: Vec<DirectMessage> = channels.into_iter().filter_map(convert_dm).collect();
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
    /// The message this one is a reply to, when it references another. Carries
    /// just enough to render the quoted preview above the message.
    pub reply: Option<MessageReference>,
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

/// Formats a Discord timestamp as `YYYY-MM-DD HH:MM` (UTC).
fn format_timestamp(timestamp: Timestamp) -> String {
    // ISO 8601 form is `2021-08-10T11:16:37.020000+00:00`.
    let iso = timestamp.iso_8601().to_string();
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => iso,
    }
}

/// Builds the CDN avatar URL for a user, when they have a custom avatar.
fn avatar_url(user: &twilight_model::user::User) -> Option<String> {
    user.avatar.map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{}/{}.webp?size=80",
            user.id, hash
        )
    })
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

fn convert_message(message: twilight_model::channel::Message) -> Message {
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
            Ok(messages
                .into_iter()
                .map(convert_message)
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}

/// A message received live over the gateway, tagged with the channel it
/// belongs to so the UI can decide whether it's for the open conversation.
pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    pub message: Message,
}

/// Opens a gateway websocket connection and invokes `on_message` for every
/// `MESSAGE_CREATE` dispatch, giving the app real-time messages for whichever
/// channel is currently open.
///
/// Runs on the background Tokio runtime; `on_message` is called from that
/// runtime's thread, not the gpui foreground thread. It returns `false` to
/// stop the connection (e.g. the receiving end went away), which ends the
/// shard loop and drops the socket.
///
/// The shard reconnects and resumes on its own, so transient errors are
/// skipped rather than treated as fatal.
pub fn connect_gateway(
    token: String,
    mut on_message: impl FnMut(IncomingMessage) -> bool + Send + 'static,
) {
    runtime_handle().spawn(async move {
        // Discord ignores the intents field for user tokens; a real user
        // client receives every event its account can see. Request all intents
        // so we mirror that and never filter events at this layer (the shard
        // still requires a value in the IDENTIFY payload).
        let mut shard = Shard::new(ShardId::ONE, token, Intents::all());

        while let Some(item) = shard.next_event(EventTypeFlags::MESSAGE_CREATE).await {
            let event = match item {
                Ok(event) => event,
                // Reconnects/resumes are handled by the shard internally; a
                // receive error just means skip this one and keep listening.
                Err(_) => continue,
            };

            if let Event::MessageCreate(message) = event {
                let MessageCreate { message, .. } = *message;
                let incoming = IncomingMessage {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                };
                if !on_message(incoming) {
                    break;
                }
            }
        }
    });
}

/// TODO: add support for higher file sizes upto 500mb with nitro checks
pub const MAX_ATTACHMENT_SIZE: u64 = 10 * 1024 * 1024;

/// Sends a message to a channel and invokes `on_done` with the created
/// message.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn send_message(
    token: String,
    channel_id: Id<ChannelMarker>,
    content: String,
    reply_to: Option<Id<MessageMarker>>,
    attachments: Vec<(String, Vec<u8>)>,
    on_done: impl FnOnce(Result<Message, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            // Attachment ids only need to be unique within this message, so the
            // slice index works.
            let attachments: Vec<Attachment> = attachments
                .into_iter()
                .enumerate()
                .map(|(index, (filename, file))| {
                    Attachment::from_bytes(filename, file, index as u64)
                })
                .collect();

            // Discord requires at least one of content/attachments; both the
            // content and attachments borrows must outlive the awaited request.
            let mut request = client.create_message(channel_id);
            if !content.is_empty() {
                request = request.content(&content);
            }
            if !attachments.is_empty() {
                request = request.attachments(&attachments);
            }
            if let Some(message_id) = reply_to {
                request = request.reply(message_id);
            }
            let response = request.await.map_err(|err| err.to_string())?;
            let message = response.model().await.map_err(|err| err.to_string())?;

            Ok(convert_message(message))
        }
        .await;

        on_done(result);
    });
}
