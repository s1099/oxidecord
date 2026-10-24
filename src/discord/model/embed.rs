//! Rich embeds: the cards Discord renders under a message's text.
//!
//! Discord serves several shapes of embed behind one API type, distinguished by
//! its `type` field. They fall into two families the UI draws very differently
//! ([`EmbedLayout`]): a bordered card with a coloured spine, and a bare piece of
//! media with no card at all.

use twilight_model::channel::message::embed::EmbedMediaFlags;

use super::cdn::{animated_webp, scaled_url, wants_animated_webp};
use super::markdown::{self, Inline, Markdown};
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
    /// Inline formatting only; the whole title is the link when there is one.
    pub title: Option<Vec<Inline>>,
    pub url: Option<String>,
    pub description: Option<Markdown>,
    pub fields: Vec<EmbedField>,
    /// The large image below the fields.
    pub image: Option<EmbedMedia>,
    /// The small square image in the card's top-right corner.
    pub thumbnail: Option<EmbedMedia>,
    /// Set when the embed points at a video; the UI overlays a play badge on
    /// the media and opens [`Self::url`] on click.
    pub has_video: bool,
    /// Set when the embed is a GIF served as video, which plays on a loop in
    /// place of [`Self::image`] — its poster frame until the clip has loaded.
    pub gif: Option<EmbedGif>,
    pub footer: Option<EmbedFooter>,
    /// Already formatted for display, appended to the footer line.
    pub timestamp: Option<String>,
    /// The embed exactly as Discord sent it, pretty-printed. Backs the debug
    /// copy button, so it is only carried in debug builds.
    #[cfg(debug_assertions)]
    pub raw: String,
}

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
    pub icon_url: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone)]
pub struct EmbedField {
    /// Inline formatting only, without links.
    pub name: Vec<Inline>,
    pub value: Markdown,
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
    /// Scaled down to the box it's drawn in.
    pub url: String,
    /// The unscaled original, which the image viewer asks for at the size of
    /// the window and opens in the browser.
    pub source: String,
    /// Intrinsic pixel dimensions, when Discord reports them. Used to lay out
    /// the exact scaled box so the message doesn't reflow once it loads.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether [`Self::source`] has to be asked of the proxy as animated WebP
    /// to stay moving at another size; see [`super::cdn::wants_animated_webp`].
    pub animated_webp: bool,
}

/// A GIF Discord sends as a muted mp4: everything from Tenor, Klipy and
/// Giphy, which arrive as `gifv` embeds. Discord's client plays these on a
/// loop with no controls, as if they were GIFs, rather than as videos.
#[derive(Clone)]
pub struct EmbedGif {
    /// The proxied clip.
    pub url: String,
    /// Intrinsic pixel dimensions of the clip, for when the poster has none.
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

    let gif = gif_for(&embed);
    let has_video = gif.is_none() && (embed.video.is_some() || embed.kind == "video");
    // Tenor and Klipy send a title and provider along with the clip, but
    // Discord draws a GIF bare whatever else the embed carries.
    let layout = if gif.is_some() {
        EmbedLayout::Media
    } else {
        layout_for(&embed)
    };

    // Link previews and video embeds carry their artwork in `thumbnail` but
    // render it large, the way `image` is drawn — Discord promotes it whenever
    // there's no real image competing for the space and it's big enough to be
    // worth it. Bare media has no corner to tuck a thumbnail into at all.
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
        // twilight doesn't carry a thumbnail's flags, so one is never known to
        // be animated.
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

/// Picks the shape an embed is drawn in. Media types only escape the card when
/// they carry nothing but the media — an image link posted on its own renders
/// bare, but one a bot wrapped in a titled embed keeps its card. GIFs served
/// as video are the exception, and never get here.
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

/// The looping clip, for an embed that is a GIF served as video: a `gifv`, or
/// any embed whose video Discord flags as animated.
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
