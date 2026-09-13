//! Rich embeds: the cards Discord renders under a message's text.
//!
//! Discord serves several shapes of embed behind one API type, distinguished by
//! its `type` field. They fall into two families the UI draws very differently
//! ([`EmbedLayout`]): a bordered card with a coloured spine, and a bare piece of
//! media with no card at all.

use super::time::format_embed_timestamp;

/// The widest the CDN is asked to scale an embed's large image to. Matches the
/// box the card lays it out in, so the full-resolution original is never
/// downloaded.
const IMAGE_MAX_WIDTH: u32 = 400;
const IMAGE_MAX_HEIGHT: u32 = 300;
/// Thumbnails sit in an 80x80 box; ask for 2x so they stay sharp.
const THUMBNAIL_REQUEST_SIZE: u32 = 160;

#[derive(Clone)]
pub struct Embed {
    /// Which of Discord's embed shapes this is, already reduced to the layout
    /// the UI draws.
    pub layout: EmbedLayout,
    /// The spine colour, as a packed `0xRRGGBB`. Embeds without one get the
    /// theme's neutral border colour instead.
    pub color: Option<u32>,
    /// The site name, shown as a small line above the author.
    pub provider: Option<String>,
    pub author: Option<EmbedAuthor>,
    pub title: Option<String>,
    /// Where the title links to, when it is a link.
    pub url: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<EmbedField>,
    /// The large image below the fields.
    pub image: Option<EmbedMedia>,
    /// The small square image in the card's top-right corner.
    pub thumbnail: Option<EmbedMedia>,
    /// Set when the embed points at a video; the UI overlays a play badge on
    /// the media and opens [`Self::url`] on click.
    pub has_video: bool,
    pub footer: Option<EmbedFooter>,
    /// Already formatted for display, appended to the footer line.
    pub timestamp: Option<String>,
    /// The embed exactly as Discord sent it, pretty-printed. Backs the debug
    /// copy button, so it is only carried in debug builds.
    #[cfg(debug_assertions)]
    pub raw: String,
}

/// The two shapes an embed is drawn in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmbedLayout {
    /// The bordered card with a coloured spine down its left edge: `rich`,
    /// `link`, `article`, and anything unrecognised.
    Card,
    /// Bare media with no card around it, the way a plain image or GIF link
    /// renders: `image`, `gifv`, and `video` embeds carrying nothing but media.
    Media,
}

#[derive(Clone)]
pub struct EmbedAuthor {
    pub name: String,
    /// The small round icon beside the name.
    pub icon_url: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    /// Whether the field shares a row with its neighbours.
    pub inline: bool,
}

#[derive(Clone)]
pub struct EmbedFooter {
    pub text: String,
    pub icon_url: Option<String>,
}

#[derive(Clone)]
pub struct EmbedMedia {
    pub url: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to lay out
    /// the exact scaled box so the message doesn't reflow once it loads.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Embed {
    /// Whether the card carries any text at all. One holding nothing but media
    /// is drawn without the content column's padding.
    pub fn has_text(&self) -> bool {
        self.provider.is_some()
            || self.author.is_some()
            || self.title.is_some()
            || self.description.is_some()
            || !self.fields.is_empty()
            || self.footer.is_some()
            || self.timestamp.is_some()
    }
}

pub(in crate::discord) fn convert_embed(embed: twilight_model::channel::message::Embed) -> Embed {
    // Serialised before anything is moved out, so the copy button hands back
    // precisely what the gateway delivered.
    #[cfg(debug_assertions)]
    let raw = serde_json::to_string_pretty(&embed).unwrap_or_else(|err| format!("<{err}>"));

    let has_video = embed.video.is_some() || embed.kind == "video" || embed.kind == "gifv";
    let layout = layout_for(&embed);

    let mut image = embed.image.map(|image| EmbedMedia {
        url: scaled_url(
            image.proxy_url.as_deref().unwrap_or(&image.url),
            image.width,
            image.height,
            IMAGE_MAX_WIDTH,
            IMAGE_MAX_HEIGHT,
        ),
        width: image.width.map(|w| w as u32),
        height: image.height.map(|h| h as u32),
    });
    let mut thumbnail = embed.thumbnail.map(|thumbnail| EmbedMedia {
        url: scaled_url(
            thumbnail.proxy_url.as_deref().unwrap_or(&thumbnail.url),
            thumbnail.width,
            thumbnail.height,
            THUMBNAIL_REQUEST_SIZE,
            THUMBNAIL_REQUEST_SIZE,
        ),
        width: thumbnail.width.map(|w| w as u32),
        height: thumbnail.height.map(|h| h as u32),
    });

    // Link previews and video embeds carry their artwork in `thumbnail` but
    // render it large, the way `image` is drawn — Discord promotes it whenever
    // there's no real image competing for the space and it's big enough to be
    // worth it. Bare media has no corner to tuck a thumbnail into at all.
    let promote_thumbnail = image.is_none()
        && match layout {
            EmbedLayout::Media => true,
            EmbedLayout::Card => {
                has_video
                    || (embed.kind == "article"
                        && thumbnail
                            .as_ref()
                            .is_some_and(|thumbnail| thumbnail.width.unwrap_or(0) >= 300))
            }
        };
    if promote_thumbnail && let Some(mut promoted) = thumbnail.take() {
        // It was sized for the 80x80 corner box; re-ask at the large size.
        promoted.url = scaled_url(
            &strip_size_query(&promoted.url),
            promoted.width.map(u64::from),
            promoted.height.map(u64::from),
            IMAGE_MAX_WIDTH,
            IMAGE_MAX_HEIGHT,
        );
        image = Some(promoted);
    }

    Embed {
        layout,
        color: embed.color,
        provider: embed
            .provider
            .and_then(|provider| provider.name)
            .filter(|name| !name.is_empty()),
        author: embed.author.map(|author| EmbedAuthor {
            name: author.name,
            icon_url: author.proxy_icon_url.or(author.icon_url),
            url: author.url,
        }),
        title: embed.title.filter(|title| !title.is_empty()),
        url: embed.url,
        description: embed
            .description
            .filter(|description| !description.is_empty()),
        fields: embed
            .fields
            .into_iter()
            .map(|field| EmbedField {
                name: field.name,
                value: field.value,
                inline: field.inline,
            })
            .collect(),
        image,
        thumbnail,
        has_video,
        footer: embed
            .footer
            .filter(|footer| !footer.text.is_empty())
            .map(|footer| EmbedFooter {
                text: footer.text,
                icon_url: footer.proxy_icon_url.or(footer.icon_url),
            }),
        timestamp: embed.timestamp.map(format_embed_timestamp),
        #[cfg(debug_assertions)]
        raw,
    }
}

/// Picks the shape an embed is drawn in. Media types only escape the card when
/// they carry nothing but the media — a GIF link posted on its own renders
/// bare, but one a bot wrapped in a titled embed keeps its card.
fn layout_for(embed: &twilight_model::channel::message::Embed) -> EmbedLayout {
    let is_media_kind = matches!(embed.kind.as_str(), "image" | "gifv" | "video");
    let has_chrome = embed.title.is_some()
        || embed.description.is_some()
        || embed.author.is_some()
        || embed.footer.is_some()
        || !embed.fields.is_empty();
    if is_media_kind && !has_chrome {
        EmbedLayout::Media
    } else {
        EmbedLayout::Card
    }
}

/// Asks the CDN for an image already scaled to the box it will be drawn in,
/// preserving its aspect ratio.
fn scaled_url(
    url: &str,
    width: Option<u64>,
    height: Option<u64>,
    max_w: u32,
    max_h: u32,
) -> String {
    // Only Discord's own proxy understands the resize query; asking a third
    // party host for it would at best be ignored and at worst break a signature.
    if !url.contains("discordapp.") && !url.contains("discord.com") {
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

/// Drops the `width`/`height` query [`scaled_url`] added, so a promoted
/// thumbnail can be re-requested at a different size without stacking duplicate
/// keys.
fn strip_size_query(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.starts_with("width=") && !pair.starts_with("height="))
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}
