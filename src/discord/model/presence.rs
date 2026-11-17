//! What someone is doing, from their presence: games, music, streams and their
//! custom status, as the full profile's Activity tab shows them.

use serde::Deserialize;

use twilight_model::id::{Id, marker::UserMarker};

use super::PresenceStatus;
use super::cdn;

#[derive(Clone, Debug, PartialEq)]
pub struct Presence {
    pub user_id: Id<UserMarker>,
    pub status: PresenceStatus,
    pub activities: Vec<Activity>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivityKind {
    Playing,
    Streaming,
    Listening,
    Watching,
    Custom,
    Competing,
}

impl ActivityKind {
    fn from_raw(kind: u8) -> Option<Self> {
        Some(match kind {
            0 => Self::Playing,
            1 => Self::Streaming,
            2 => Self::Listening,
            3 => Self::Watching,
            4 => Self::Custom,
            5 => Self::Competing,
            _ => return None,
        })
    }

    /// The card's heading, as Discord words it.
    pub fn heading(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Streaming => "Live on",
            Self::Listening => "Listening to",
            Self::Watching => "Watching",
            Self::Custom => "Custom Status",
            Self::Competing => "Competing in",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub kind: ActivityKind,
    pub name: String,
    pub app_id: Option<u64>,
    pub details: Option<String>,
    pub state: Option<String>,
    /// Unix milliseconds.
    pub started_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// A custom status's emoji: a custom one's image, or a unicode one's text.
    pub emoji: Option<ActivityEmoji>,
    /// Where a stream is watched.
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActivityEmoji {
    Custom(String),
    Unicode(String),
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawActivity {
    #[serde(rename = "type", default)]
    kind: u8,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    application_id: Option<serde_json::Value>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    timestamps: Option<RawTimestamps>,
    #[serde(default)]
    assets: Option<RawAssets>,
    #[serde(default)]
    emoji: Option<RawActivityEmoji>,
}

#[derive(Deserialize)]
struct RawTimestamps {
    #[serde(default)]
    start: Option<serde_json::Value>,
    #[serde(default)]
    end: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawAssets {
    #[serde(default)]
    large_image: Option<String>,
    #[serde(default)]
    large_text: Option<String>,
    #[serde(default)]
    small_image: Option<String>,
    #[serde(default)]
    small_text: Option<String>,
}

#[derive(Deserialize)]
struct RawActivityEmoji {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    animated: bool,
}

/// Snowflakes and timestamps come as strings or numbers depending on the client
/// that set the activity.
fn number(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn non_empty(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.trim().is_empty())
}

pub(in crate::discord) fn convert_activities(raw: Vec<RawActivity>) -> Vec<Activity> {
    raw.into_iter().filter_map(convert_activity).collect()
}

fn convert_activity(raw: RawActivity) -> Option<Activity> {
    let kind = ActivityKind::from_raw(raw.kind)?;
    let app_id = raw
        .application_id
        .as_ref()
        .and_then(number)
        .and_then(|id| u64::try_from(id).ok());
    let assets = raw.assets;
    let image = |key: Option<&String>| key.and_then(|key| asset_url(app_id, key));
    let emoji = raw.emoji.and_then(|emoji| match emoji.id {
        Some(id) => Some(ActivityEmoji::Custom(cdn::emoji_url(id, emoji.animated))),
        None => emoji.name.map(ActivityEmoji::Unicode),
    });
    let activity = Activity {
        kind,
        name: raw.name.unwrap_or_default(),
        app_id,
        details: non_empty(raw.details),
        state: non_empty(raw.state),
        started_at: raw
            .timestamps
            .as_ref()
            .and_then(|stamps| stamps.start.as_ref())
            .and_then(number),
        ends_at: raw
            .timestamps
            .as_ref()
            .and_then(|stamps| stamps.end.as_ref())
            .and_then(number),
        large_image: image(
            assets
                .as_ref()
                .and_then(|assets| assets.large_image.as_ref()),
        ),
        large_text: non_empty(assets.as_ref().and_then(|assets| assets.large_text.clone())),
        small_image: image(
            assets
                .as_ref()
                .and_then(|assets| assets.small_image.as_ref()),
        ),
        small_text: non_empty(assets.and_then(|assets| assets.small_text)),
        emoji,
        url: raw.url,
    };
    // An empty custom status still arrives; there's nothing to show for it.
    (kind != ActivityKind::Custom || activity.state.is_some() || activity.emoji.is_some())
        .then_some(activity)
}

/// Rich presence images are application asset ids, or prefixed keys for images
/// hosted elsewhere.
fn asset_url(app_id: Option<u64>, key: &str) -> Option<String> {
    if let Some(path) = key.strip_prefix("mp:") {
        let url = format!("https://media.discordapp.net/{path}");
        // As Discord's client asks, so animated ones keep moving.
        let lower = path.to_ascii_lowercase();
        return Some(if lower.ends_with(".gif") {
            format!("{url}?format=webp&animated=true")
        } else if lower.ends_with(".webp") {
            format!("{url}?animated=true")
        } else {
            url
        });
    }
    if let Some(id) = key.strip_prefix("spotify:") {
        return Some(format!("https://i.scdn.co/image/{id}"));
    }
    if let Some(id) = key.strip_prefix("youtube:") {
        return Some(format!("https://i.ytimg.com/vi/{id}/hqdefault_live.jpg"));
    }
    if let Some(name) = key.strip_prefix("twitch:") {
        return Some(format!(
            "https://static-cdn.jtvnw.net/previews-ttv/live_user_{name}-162x90.jpg"
        ));
    }
    if key.contains(':') {
        return None;
    }
    app_id.map(|app_id| cdn::app_asset_url(app_id, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_and_game_activities_convert() {
        let raw: Vec<RawActivity> = serde_json::from_value(serde_json::json!([
            {"type": 4, "name": "Custom Status", "state": ""},
            {"type": 2, "name": "Spotify", "details": "Song", "state": "Artist",
             "assets": {"large_image": "spotify:abc", "large_text": "Album"},
             "timestamps": {"start": 1000, "end": "2000"}},
            {"type": 0, "name": "Game", "application_id": "42",
             "assets": {"large_image": "123"}},
        ]))
        .unwrap();
        let activities = convert_activities(raw);
        assert_eq!(activities.len(), 2);
        assert_eq!(
            activities[0].large_image.as_deref(),
            Some("https://i.scdn.co/image/abc")
        );
        assert_eq!(activities[0].ends_at, Some(2000));
        assert_eq!(
            activities[1].large_image.as_deref(),
            Some("https://cdn.discordapp.com/app-assets/42/123.png")
        );
    }
}
