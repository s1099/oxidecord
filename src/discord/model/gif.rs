//! GIFs for the picker: Tenor's results, and the user's favorites from Discord's
//! `FrecencyUserSettings` (`settings-proto/2`).
//!
//! Field 2 is `FavoriteGIFs`: `gifs` (1), a `map<string, FavoriteGIF>` keyed by the
//! GIF's page URL, and `hide_tooltip` (2). A `FavoriteGIF` holds `format` (1),
//! `src` (2, the media URL), `width` (3), `height` (4) and `order` (5).

use serde::Deserialize;

use super::settings::{
    Fields, Value, base64_decode, base64_encode, length_delimited, write_varint,
};

/// A Tenor result.
#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    /// The tenor.com page. Sent as the message, which Discord unfurls into a
    /// `gifv` embed, and the key a favorite is saved under.
    pub url: String,
    /// The full-size mp4, saved as a favorite's media.
    pub src: String,
    /// A small mp4 for the picker's tiles.
    pub preview: String,
    /// A still of the first frame, drawn until the clip loads.
    pub poster: String,
    pub width: u32,
    pub height: u32,
    pub title: String,
}

/// One of Tenor's featured search terms, shown as a tile before anything is typed.
#[derive(Clone, Debug)]
pub struct GifCategory {
    pub term: String,
    pub name: String,
    /// A still for the tile.
    pub image: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GifFormat {
    /// An animated image file, which gpui decodes itself.
    Image,
    #[default]
    Video,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FavoriteGif {
    /// The page URL: what's sent, and the map's key.
    pub url: String,
    pub src: String,
    pub format: GifFormat,
    pub width: u32,
    pub height: u32,
    /// Discord's client lists the highest first.
    pub order: u32,
}

impl FavoriteGif {
    /// Words to search a favorite by. A favorite keeps no title, but Tenor's page
    /// URLs end in one (`/view/handshake-cat-meme-gif-391…`).
    pub fn matches(&self, query: &str) -> bool {
        let haystack = self
            .url
            .rsplit('/')
            .find(|segment| !segment.is_empty())
            .unwrap_or(&self.url)
            .replace(['-', '_'], " ")
            .to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FavoriteGifs {
    pub gifs: Vec<FavoriteGif>,
    /// Kept only so a save doesn't reset it for Discord's client.
    pub hide_tooltip: bool,
}

impl FavoriteGifs {
    pub fn contains(&self, url: &str) -> bool {
        self.gifs.iter().any(|gif| gif.url == url)
    }

    /// Newest first, as Discord's client lists them.
    pub fn sorted(&self) -> Vec<FavoriteGif> {
        let mut gifs = self.gifs.clone();
        gifs.sort_by_key(|gif| std::cmp::Reverse(gif.order));
        gifs
    }

    pub fn add(&mut self, mut gif: FavoriteGif) {
        if self.contains(&gif.url) {
            return;
        }
        gif.order = self.gifs.iter().map(|gif| gif.order).max().unwrap_or(0) + 1;
        self.gifs.push(gif);
    }

    pub fn remove(&mut self, url: &str) {
        self.gifs.retain(|gif| gif.url != url);
    }
}

/// `None` when the blob doesn't carry field 2, as a partial gateway update about
/// another field doesn't.
pub(in crate::discord) fn parse_favorite_gifs(settings: &str) -> Option<FavoriteGifs> {
    let bytes = base64_decode(settings)?;
    let mut found = None;
    for (number, value) in Fields::new(&bytes) {
        if let (2, Value::Bytes(bytes)) = (number, value) {
            found = Some(parse_favorites(bytes));
        }
    }
    found
}

fn parse_favorites(bytes: &[u8]) -> FavoriteGifs {
    let mut favorites = FavoriteGifs::default();
    for (number, value) in Fields::new(bytes) {
        match (number, value) {
            (1, Value::Bytes(entry)) => favorites.gifs.extend(parse_entry(entry)),
            (2, Value::Varint(hide)) => favorites.hide_tooltip = hide != 0,
            _ => {}
        }
    }
    favorites
}

fn parse_entry(bytes: &[u8]) -> Option<FavoriteGif> {
    let mut url = None;
    let mut gif = FavoriteGif {
        url: String::new(),
        src: String::new(),
        format: GifFormat::Video,
        width: 0,
        height: 0,
        order: 0,
    };
    for (number, value) in Fields::new(bytes) {
        match (number, value) {
            (1, Value::Bytes(key)) => url = std::str::from_utf8(key).ok().map(str::to_owned),
            (2, Value::Bytes(value)) => {
                for (number, value) in Fields::new(value) {
                    match (number, value) {
                        (1, Value::Varint(1)) => gif.format = GifFormat::Image,
                        (2, Value::Bytes(src)) => {
                            gif.src = String::from_utf8_lossy(src).into_owned()
                        }
                        (3, Value::Varint(width)) => gif.width = width as u32,
                        (4, Value::Varint(height)) => gif.height = height as u32,
                        (5, Value::Varint(order)) => gif.order = order as u32,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    gif.url = url.filter(|url| !url.is_empty())?;
    (!gif.src.is_empty()).then_some(gif)
}

/// Encodes a `FrecencyUserSettings` holding only `favorite_gifs`, for
/// `PATCH /users/@me/settings-proto/2`. A top-level field sent replaces the one
/// stored, so this is every favorite, not a change.
pub(in crate::discord) fn encode_favorite_gifs(favorites: &FavoriteGifs) -> String {
    let mut message = Vec::new();
    for gif in &favorites.gifs {
        let mut value = Vec::new();
        write_varint(&mut value, 1 << 3);
        write_varint(
            &mut value,
            match gif.format {
                GifFormat::Image => 1,
                GifFormat::Video => 2,
            },
        );
        value.extend(length_delimited(2, gif.src.as_bytes()));
        for (number, field) in [(3, gif.width), (4, gif.height), (5, gif.order)] {
            write_varint(&mut value, number << 3);
            write_varint(&mut value, u64::from(field));
        }
        let mut entry = length_delimited(1, gif.url.as_bytes());
        entry.extend(length_delimited(2, &value));
        message.extend(length_delimited(1, &entry));
    }
    if favorites.hide_tooltip {
        write_varint(&mut message, 2 << 3);
        write_varint(&mut message, 1);
    }
    base64_encode(&length_delimited(2, &message))
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawTenorResults {
    pub results: Vec<RawTenorGif>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawTenorGif {
    itemurl: String,
    #[serde(default)]
    content_description: String,
    #[serde(default)]
    h1_title: String,
    media: Vec<RawTenorMedia>,
}

#[derive(Deserialize)]
struct RawTenorMedia {
    mp4: Option<RawTenorFile>,
    tinymp4: Option<RawTenorFile>,
    tinygif: Option<RawTenorFile>,
}

#[derive(Deserialize)]
struct RawTenorFile {
    url: String,
    #[serde(default)]
    dims: Vec<u32>,
    #[serde(default)]
    preview: String,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawTenorCategories {
    pub tags: Vec<RawTenorCategory>,
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawTenorCategory {
    searchterm: String,
    name: String,
    image: String,
}

pub(in crate::discord) fn convert_tenor_gif(raw: RawTenorGif) -> Option<Gif> {
    let media = raw.media.into_iter().next()?;
    let mp4 = media.mp4?;
    let (width, height) = match mp4.dims.as_slice() {
        [width, height, ..] if *width > 0 && *height > 0 => (*width, *height),
        _ => return None,
    };
    let preview = media
        .tinymp4
        .map(|file| file.url)
        .unwrap_or(mp4.url.clone());
    let poster = media
        .tinygif
        .map(|file| file.preview)
        .filter(|preview| !preview.is_empty())
        .unwrap_or(mp4.preview);
    let title = [raw.h1_title, raw.content_description]
        .into_iter()
        .find(|title| !title.is_empty())
        .unwrap_or_default();
    Some(Gif {
        url: raw.itemurl,
        src: mp4.url,
        preview,
        poster,
        width,
        height,
        title,
    })
}

pub(in crate::discord) fn convert_tenor_category(raw: RawTenorCategory) -> GifCategory {
    GifCategory {
        name: raw.name.trim_start_matches('#').to_owned(),
        term: raw.searchterm,
        // The tile's image is a whole tiny GIF; its first frame is served as a PNG at
        // the same path, under the size code Tenor gives previews.
        image: raw
            .image
            .replacen("AAAAM/", "AAAAN/", 1)
            .replace(".gif", ".png"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favorite(url: &str, order: u32) -> FavoriteGif {
        FavoriteGif {
            url: url.into(),
            src: format!("{url}.mp4"),
            format: GifFormat::Video,
            width: 220,
            height: 124,
            order,
        }
    }

    #[test]
    fn round_trips_through_a_patch() {
        let favorites = FavoriteGifs {
            gifs: vec![
                favorite("https://tenor.com/view/cat-gif-1", 1),
                favorite("https://tenor.com/view/dog-gif-2", 2),
            ],
            hide_tooltip: true,
        };
        let parsed = parse_favorite_gifs(&encode_favorite_gifs(&favorites)).unwrap();
        assert_eq!(parsed, favorites);
    }

    #[test]
    fn reads_a_tenor_result() {
        let raw: RawTenorResults = serde_json::from_str(
            r#"{"results":[{"id":"1","h1_title":"Handshake Cat Meme GIF",
            "itemurl":"https://tenor.com/view/handshake-cat-meme-gif-1",
            "media":[{"mp4":{"url":"https://media.tenor.com/a/x.mp4","dims":[276,498],
            "preview":"https://media.tenor.com/a/x.png"},
            "tinymp4":{"url":"https://media.tenor.com/b/x.mp4","dims":[138,248]},
            "tinygif":{"url":"https://media.tenor.com/c/x.gif","dims":[165,299],
            "preview":"https://media.tenor.com/d/x.png"}}]}],"next":"1"}"#,
        )
        .unwrap();
        let gif = convert_tenor_gif(raw.results.into_iter().next().unwrap()).unwrap();
        assert_eq!(gif.url, "https://tenor.com/view/handshake-cat-meme-gif-1");
        assert_eq!(gif.src, "https://media.tenor.com/a/x.mp4");
        assert_eq!(gif.preview, "https://media.tenor.com/b/x.mp4");
        assert_eq!(gif.poster, "https://media.tenor.com/d/x.png");
        assert_eq!((gif.width, gif.height), (276, 498));
    }

    #[test]
    fn absent_without_the_field() {
        assert_eq!(parse_favorite_gifs(&base64_encode(&[])), None);
    }

    #[test]
    fn searches_the_page_slug() {
        let gif = favorite("https://tenor.com/view/handshake-cat-meme-gif-391", 1);
        assert!(gif.matches("cat hand"));
        assert!(!gif.matches("dog"));
    }

    #[test]
    fn adds_newest_on_top() {
        let mut favorites = FavoriteGifs::default();
        favorites.add(favorite("a", 0));
        favorites.add(favorite("b", 0));
        favorites.add(favorite("a", 0));
        let sorted = favorites.sorted();
        assert_eq!(sorted.len(), 2);
        assert_eq!(sorted[0].url, "b");
    }
}
