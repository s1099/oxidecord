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

/// Asks the CDN for the image pre-scaled to fit the box, keeping aspect ratio.
pub fn scaled_url(
    url: &str,
    width: Option<u64>,
    height: Option<u64>,
    max_w: u32,
    max_h: u32,
) -> String {
    // Third-party hosts would ignore the resize query or break a signature.
    if !is_discord_proxy(url) {
        return url.to_string();
    }
    let (target_w, target_h) = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let scale = (f64::from(max_w) / w as f64)
                .min(f64::from(max_h) / h as f64)
                .min(1.0);
            (
                (w as f64 * scale).round().max(1.0) as u32,
                (h as f64 * scale).round().max(1.0) as u32,
            )
        }
        _ => (max_w, max_h),
    };
    // A proxy URL already carries a signed query string, so append with `&`.
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}width={target_w}&height={target_h}")
}

/// Whether an animated image must be requested as animated WebP to keep moving.
/// GIFs don't: the proxy scales them as GIF. Other animated formats (e.g. the
/// AVIF vxtwitter serves) get converted, and WebP is the only other animated
/// format gpui decodes.
pub(super) fn wants_animated_webp(animated: bool, content_type: Option<&str>) -> bool {
    animated && content_type != Some("image/gif")
}

/// Asks the proxy for `url` as animated WebP; see [`wants_animated_webp`].
pub fn animated_webp(url: String) -> String {
    if !is_discord_proxy(&url) {
        return url;
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}format=webp&animated=true")
}

fn is_discord_proxy(url: &str) -> bool {
    url.contains("discordapp.") || url.contains("discord.com")
}
