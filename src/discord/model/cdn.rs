//! URLs for the images Discord serves off its CDN.

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

/// Extension for a CDN image hash: animated assets are prefixed `a_` and only
/// animate as GIF, while static ones are smaller as webp.
fn extension(hash: &str) -> &'static str {
    if hash.starts_with("a_") {
        "gif"
    } else {
        "webp"
    }
}

pub(super) fn avatar_url(user_id: u64, hash: &str, size: u32) -> String {
    format!(
        "https://cdn.discordapp.com/avatars/{user_id}/{hash}.{}?size={size}",
        extension(hash)
    )
}

pub(super) fn banner_url(user_id: u64, hash: &str, size: u32) -> String {
    format!(
        "https://cdn.discordapp.com/banners/{user_id}/{hash}.{}?size={size}",
        extension(hash)
    )
}

/// The avatar shown beside a name in a list or a message, always static.
pub(super) fn small_avatar_url(user_id: u64, hash: &str) -> String {
    format!("https://cdn.discordapp.com/avatars/{user_id}/{hash}.webp?size=80")
}

pub(super) fn guild_icon_url(guild_id: Id<GuildMarker>, hash: &str) -> String {
    format!("https://cdn.discordapp.com/icons/{guild_id}/{hash}.webp?size=100&quality=lossless")
}

pub(super) fn group_dm_icon_url(channel_id: Id<ChannelMarker>, hash: &str) -> String {
    format!("https://cdn.discordapp.com/channel-icons/{channel_id}/{hash}.webp?size=80")
}

pub(super) fn emoji_url(id: impl std::fmt::Display, animated: bool) -> String {
    format!(
        "https://cdn.discordapp.com/emojis/{id}.{}?size=44",
        if animated { "gif" } else { "webp" }
    )
}
