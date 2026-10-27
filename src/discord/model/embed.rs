//! Rich embeds. Discord's several `type`s reduce to two layouts ([`EmbedLayout`]):
//! a bordered card, and bare media with no card.

use twilight_model::channel::message::embed::EmbedMediaFlags;

use super::cdn::{animated_webp, scaled_url, wants_animated_webp};
use super::markdown::{self, Inline, Markdown};
use super::time::format_embed_timestamp;

/// Matches the card's image box, so the full-resolution original is never downloaded.
const IMAGE_MAX_WIDTH: u32 = 400;
const IMAGE_MAX_HEIGHT: u32 = 300;
/// Thumbnails sit in an 80x80 box; 2x keeps them sharp.
const THUMBNAIL_REQUEST_SIZE: u32 = 160;

#[derive(Clone)]
pub struct Embed {
    pub layout: EmbedLayout,
    /// Spine colour, packed `0xRRGGBB`; the theme's border colour when absent.
    pub color: Option<u32>,
    pub provider: Option<String>,
    pub author: Option<EmbedAuthor>,
    /// Inline formatting only; the whole title is the link when there is one.
    pub title: Option<Vec<Inline>>,
    pub url: Option<String>,
    pub description: Option<Markdown>,
    pub fields: Vec<EmbedField>,
    pub image: Option<EmbedMedia>,
    pub thumbnail: Option<EmbedMedia>,
    /// The UI overlays a play badge and opens [`Self::url`] on click.
    pub has_video: bool,
    /// A GIF served as video; loops in place of [`Self::image`], its poster
    /// frame until the clip loads.
    pub gif: Option<EmbedGif>,
    pub footer: Option<EmbedFooter>,
    pub timestamp: Option<String>,
    /// The embed as Discord sent it, for the debug copy button.
    #[cfg(debug_assertions)]
    pub raw: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmbedLayout {
    /// `rich`, `link`, `article`, and anything unrecognised.
    Card,
    /// `image`, `gifv`, and `video` embeds carrying nothing but media.
    Media,
}

#[derive(Clone)]
pub struct EmbedAuthor {
    pub name: String,
    pub icon_url: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone)]
pub struct EmbedField {
    /// Inline formatting only, without links.
    pub name: Vec<Inline>,
    pub value: Markdown,
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
    /// The unscaled original, for the image viewer and the browser.
    pub source: String,
    /// Intrinsic pixels, so the box is laid out before load without reflow.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether [`Self::source`] must be requested as animated WebP to stay
    /// moving at another size; see [`super::cdn::wants_animated_webp`].
    pub animated_webp: bool,
}

/// A GIF sent as a muted mp4 (Tenor, Klipy, Giphy `gifv` embeds), played on a
/// loop without controls.
#[derive(Clone)]
pub struct EmbedGif {
    pub url: String,
    /// For when the poster has no dimensions.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Embed {
    /// A card with nothing but media is drawn without the content padding.
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
    // Serialised before anything is moved out.
    #[cfg(debug_assertions)]
    let raw = serde_json::to_string_pretty(&embed).unwrap_or_else(|err| format!("<{err}>"));

    let gif = gif_for(&embed);
    let has_video = gif.is_none() && (embed.video.is_some() || embed.kind == "video");
    // Tenor and Klipy send a title and provider too, but Discord draws a GIF bare.
    let layout = if gif.is_some() {
        EmbedLayout::Media
    } else {
        layout_for(&embed)
    };

    // Link previews and videos carry artwork in `thumbnail` but Discord draws
    // it large when no real image competes and it's big enough. Bare media has
    // no corner for a thumbnail.
    let promote_thumbnail = embed.image.is_none()
        && match layout {
            EmbedLayout::Media => true,
            EmbedLayout::Card => {
                has_video
                    || (embed.kind == "article"
                        && embed
                            .thumbnail
                            .as_ref()
                            .is_some_and(|thumbnail| thumbnail.width.unwrap_or(0) >= 300))
            }
        };

    let large = |url: &str, width, height, as_webp| {
        let scaled = scaled_url(url, width, height, IMAGE_MAX_WIDTH, IMAGE_MAX_HEIGHT);
        EmbedMedia {
            url: if as_webp {
                animated_webp(scaled)
            } else {
                scaled
            },
            source: url.to_owned(),
            width: width.map(|w| w as u32),
            height: height.map(|h| h as u32),
            animated_webp: as_webp,
        }
    };
    let mut image = embed.image.map(|image| {
        let animated = image
            .flags
            .is_some_and(|flags| flags.contains(EmbedMediaFlags::IS_ANIMATED));
        large(
            image.proxy_url.as_deref().unwrap_or(&image.url),
            image.width,
            image.height,
            wants_animated_webp(animated, image.content_type.as_deref()),
        )
    });
    let mut thumbnail = None;
    if let Some(raw) = embed.thumbnail {
        let url = raw.proxy_url.as_deref().unwrap_or(&raw.url);
        // twilight doesn't carry a thumbnail's flags, so it's never known animated.
        if promote_thumbnail {
            image = Some(large(url, raw.width, raw.height, false));
        } else {
            thumbnail = Some(EmbedMedia {
                url: scaled_url(
                    url,
                    raw.width,
                    raw.height,
                    THUMBNAIL_REQUEST_SIZE,
                    THUMBNAIL_REQUEST_SIZE,
                ),
                source: url.to_owned(),
                width: raw.width.map(|w| w as u32),
                height: raw.height.map(|h| h as u32),
                animated_webp: false,
            });
        }
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
        title: embed
            .title
            .filter(|title| !title.is_empty())
            .map(|title| markdown::parse_inline(&title)),
        url: embed.url,
        description: embed
            .description
            .filter(|description| !description.is_empty())
            .map(|description| Markdown::parse(&description)),
        fields: embed
            .fields
            .into_iter()
            .map(|field| EmbedField {
                name: markdown::parse_inline(&field.name),
                value: Markdown::parse(&field.value),
                inline: field.inline,
            })
            .collect(),
        image,
        thumbnail,
        has_video,
        gif,
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

/// Media kinds only drop the card when they carry nothing else: a bare image
/// link renders bare, a bot's titled image embed keeps its card.
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

/// For a `gifv`, or any embed whose video Discord flags as animated.
fn gif_for(embed: &twilight_model::channel::message::Embed) -> Option<EmbedGif> {
    let video = embed.video.as_ref()?;
    let animated = video
        .flags
        .is_some_and(|flags| flags.contains(EmbedMediaFlags::IS_ANIMATED));
    if embed.kind != "gifv" && !animated {
        return None;
    }
    Some(EmbedGif {
        url: video.proxy_url.clone().or_else(|| video.url.clone())?,
        width: video.width.map(|w| w as u32),
        height: video.height.map(|h| h as u32),
    })
}
