//! Stickers: those a message carries, and those the sticker picker offers from
//! the user's guilds and Discord's own packs.

use serde::Deserialize;
use twilight_model::channel::message::sticker::{MessageSticker, StickerFormatType};
use twilight_model::id::{
    Id,
    marker::{StickerMarker, StickerPackMarker},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StickerFormat {
    Png,
    Apng,
    /// Vector JSON, rasterized here; see [`crate::platform::lottie`].
    Lottie,
    Gif,
}

impl From<StickerFormatType> for StickerFormat {
    fn from(format: StickerFormatType) -> Self {
        match format {
            StickerFormatType::Apng => Self::Apng,
            StickerFormatType::Lottie => Self::Lottie,
            StickerFormatType::Gif => Self::Gif,
            // An unknown format is most likely still an image the proxy can serve.
            _ => Self::Png,
        }
    }
}

/// What a message carries of a sticker, and enough to draw it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StickerItem {
    pub id: Id<StickerMarker>,
    pub name: String,
    pub format: StickerFormat,
}

impl StickerItem {
    /// `None` for Lottie, which [`Self::lottie_url`] serves instead. A still is
    /// the first frame, for the picker, where only the hovered one moves.
    pub fn image_url(&self, animated: bool, size: u32) -> Option<String> {
        let id = self.id;
        let base = "https://media.discordapp.net/stickers";
        Some(match self.format {
            StickerFormat::Lottie => return None,
            // APNG doesn't animate in gpui, so it's asked for as animated WebP.
            StickerFormat::Apng if animated => {
                format!("{base}/{id}.webp?size={size}&animated=true")
            }
            // The CDN doesn't serve GIF stickers; only the proxy does.
            StickerFormat::Gif if animated => format!("{base}/{id}.gif?size={size}"),
            _ => format!("{base}/{id}.webp?size={size}"),
        })
    }

    pub fn lottie_url(&self) -> String {
        format!("https://cdn.discordapp.com/stickers/{}.json", self.id)
    }

    pub fn animates(&self) -> bool {
        self.format != StickerFormat::Png
    }
}

/// A sticker the picker can offer.
#[derive(Clone, Debug)]
pub struct Sticker {
    pub item: StickerItem,
    /// Comma-separated words and emoji the search also matches.
    pub tags: String,
    /// False once the guild loses the boost level that paid for the slot.
    pub available: bool,
}

impl Sticker {
    pub fn matches(&self, query: &str) -> bool {
        self.item.name.to_lowercase().contains(query) || self.tags.to_lowercase().contains(query)
    }
}

/// One of Discord's own packs, free to everyone.
#[derive(Clone, Debug)]
pub struct StickerPack {
    pub id: Id<StickerPackMarker>,
    pub name: String,
    /// Stands for the pack in the picker's rail.
    pub cover: Option<StickerItem>,
    pub stickers: Vec<Sticker>,
}

/// A sticker as `READY`, `GUILD_CREATE`, `GUILD_STICKERS_UPDATE` and
/// `GET /sticker-packs` send it.
#[derive(Deserialize)]
pub(in crate::discord) struct RawSticker {
    id: Id<StickerMarker>,
    name: String,
    format_type: StickerFormatType,
    #[serde(default)]
    tags: Option<String>,
    #[serde(default = "available_default")]
    available: bool,
    #[serde(default)]
    sort_value: Option<u64>,
}

fn available_default() -> bool {
    true
}

#[derive(Deserialize)]
pub(in crate::discord) struct RawStickerPacks {
    sticker_packs: Vec<RawStickerPack>,
}

#[derive(Deserialize)]
struct RawStickerPack {
    id: Id<StickerPackMarker>,
    name: String,
    #[serde(default)]
    cover_sticker_id: Option<Id<StickerMarker>>,
    stickers: Vec<RawSticker>,
}

pub(in crate::discord) fn convert_sticker(sticker: RawSticker) -> Sticker {
    Sticker {
        item: StickerItem {
            id: sticker.id,
            name: sticker.name,
            format: sticker.format_type.into(),
        },
        tags: sticker.tags.unwrap_or_default(),
        available: sticker.available,
    }
}

/// In the order Discord's client lists them.
pub(in crate::discord) fn convert_sticker_packs(raw: RawStickerPacks) -> Vec<StickerPack> {
    raw.sticker_packs
        .into_iter()
        .map(|pack| {
            let mut stickers = pack.stickers;
            stickers.sort_by_key(|sticker| sticker.sort_value);
            let stickers: Vec<Sticker> = stickers.into_iter().map(convert_sticker).collect();
            let cover = pack
                .cover_sticker_id
                .and_then(|id| stickers.iter().find(|sticker| sticker.item.id == id))
                .or(stickers.first())
                .map(|sticker| sticker.item.clone());
            StickerPack {
                id: pack.id,
                name: pack.name,
                cover,
                stickers,
            }
        })
        .collect()
}

pub(super) fn convert_sticker_items(items: Vec<MessageSticker>) -> Vec<StickerItem> {
    items
        .into_iter()
        .map(|item| StickerItem {
            id: item.id,
            name: item.name,
            format: item.format_type.into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_sort_and_find_their_cover() {
        let raw: RawStickerPacks = serde_json::from_str(
            r#"{"sticker_packs": [{
                "id": "1", "name": "Wumpus", "sku_id": "2", "cover_sticker_id": "11",
                "description": "", "stickers": [
                    {"id": "11", "name": "Wave", "format_type": 3, "type": 1, "tags": "hi", "sort_value": 2, "description": "", "asset": ""},
                    {"id": "10", "name": "Nod", "format_type": 2, "type": 1, "tags": "yes", "sort_value": 1}
                ]
            }]}"#,
        )
        .unwrap();
        let packs = convert_sticker_packs(raw);
        let names: Vec<_> = packs[0]
            .stickers
            .iter()
            .map(|s| s.item.name.as_str())
            .collect();
        assert_eq!(names, ["Nod", "Wave"]);
        let cover = packs[0].cover.as_ref().unwrap();
        assert_eq!(cover.format, StickerFormat::Lottie);
        assert_eq!(cover.name, "Wave");
        assert_eq!(packs[0].stickers[1].tags, "hi");
    }

    #[test]
    fn urls_follow_the_format() {
        let sticker = |format| StickerItem {
            id: Id::new(5),
            name: "s".into(),
            format,
        };
        assert_eq!(sticker(StickerFormat::Lottie).image_url(true, 160), None);
        assert_eq!(
            sticker(StickerFormat::Apng).image_url(true, 160).unwrap(),
            "https://media.discordapp.net/stickers/5.webp?size=160&animated=true"
        );
        assert_eq!(
            sticker(StickerFormat::Apng).image_url(false, 160).unwrap(),
            "https://media.discordapp.net/stickers/5.webp?size=160"
        );
        assert!(
            sticker(StickerFormat::Gif)
                .image_url(true, 160)
                .unwrap()
                .contains("5.gif")
        );
    }
}
