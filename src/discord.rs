use std::sync::OnceLock;

use serde::Deserialize;
use tokio::runtime::Handle;
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

#[derive(Clone)]
pub struct Message {
    pub id: Id<MessageMarker>,
    pub author_id: Id<UserMarker>,
    pub author_name: String,
    pub author_avatar_url: Option<String>,
    pub content: String,
    pub timestamp: String,
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
    let author_name = message
        .author
        .global_name
        .unwrap_or_else(|| message.author.name.clone());

    Message {
        id: message.id,
        author_id: message.author.id,
        author_name,
        author_avatar_url,
        content: message.content,
        timestamp: format_timestamp(message.timestamp),
    }
}

/// Fetches the most recent messages in a channel (oldest first) and invokes
/// `on_done` with the result.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn fetch_messages(
    token: String,
    channel_id: Id<ChannelMarker>,
    on_done: impl FnOnce(Result<Vec<Message>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .channel_messages(channel_id)
                .limit(50)
                .await
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
