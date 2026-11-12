//! The sticker picker's contents: recently sent stickers, then the user's guilds'
//! and Discord's own packs, narrowed by a search. The caller decides which
//! stickers may be sent here.

use twilight_model::id::{
    Id,
    marker::{GuildMarker, StickerPackMarker},
};

use crate::discord::{Sticker, StickerItem};

pub(in crate::screens::home) const COLUMNS: usize = 4;

pub(in crate::screens::home) const RECENT_LIMIT: usize = COLUMNS * 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(in crate::screens::home) enum SectionKind {
    Recent,
    Guild(Id<GuildMarker>),
    Pack(Id<StickerPackMarker>),
}

#[derive(Clone)]
pub(in crate::screens::home) enum SectionIcon {
    Recent,
    /// A guild's icon, or its initials without one.
    Guild(Option<String>),
    /// A pack is shown by one of its stickers.
    Sticker(StickerItem),
}

pub(in crate::screens::home) struct PickerSection {
    pub kind: SectionKind,
    pub title: String,
    pub icon: SectionIcon,
    pub stickers: Vec<Sticker>,
}

pub(in crate::screens::home) struct SectionHeader {
    pub title: String,
    pub icon: SectionIcon,
    pub first_row: usize,
}

pub(in crate::screens::home) enum PickerRow {
    Header(usize),
    Stickers(Vec<Sticker>),
}

#[derive(Default)]
pub(in crate::screens::home) struct PickerLayout {
    pub sections: Vec<SectionHeader>,
    pub rows: Vec<PickerRow>,
}

impl PickerLayout {
    pub fn section_at(&self, row: usize) -> Option<usize> {
        self.sections
            .iter()
            .rposition(|section| section.first_row <= row)
    }
}

/// A search drops the recents, which would only repeat matches, and empty sections.
pub(in crate::screens::home) fn picker_rows(
    sections: impl IntoIterator<Item = PickerSection>,
    query: &str,
) -> PickerLayout {
    let query = query.trim().to_lowercase();
    let mut layout = PickerLayout::default();
    for mut section in sections {
        if !query.is_empty() {
            if section.kind == SectionKind::Recent {
                continue;
            }
            section.stickers.retain(|sticker| sticker.matches(&query));
        }
        if section.stickers.is_empty() {
            continue;
        }
        layout.rows.push(PickerRow::Header(layout.sections.len()));
        layout.sections.push(SectionHeader {
            title: section.title,
            icon: section.icon,
            first_row: layout.rows.len() - 1,
        });
        layout.rows.extend(
            section
                .stickers
                .chunks(COLUMNS)
                .map(|chunk| PickerRow::Stickers(chunk.to_vec())),
        );
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::StickerFormat;

    fn sticker(id: u64, name: &str, tags: &str) -> Sticker {
        Sticker {
            item: StickerItem {
                id: Id::new(id),
                name: name.into(),
                format: StickerFormat::Png,
            },
            tags: tags.into(),
            available: true,
        }
    }

    fn section(kind: SectionKind, stickers: Vec<Sticker>) -> PickerSection {
        PickerSection {
            kind,
            title: String::new(),
            icon: SectionIcon::Recent,
            stickers,
        }
    }

    #[test]
    fn rows_chunk_by_columns() {
        let stickers = (1..=5).map(|id| sticker(id, "s", "")).collect();
        let layout = picker_rows([section(SectionKind::Pack(Id::new(1)), stickers)], "");
        assert!(matches!(layout.rows[0], PickerRow::Header(0)));
        assert!(matches!(&layout.rows[1], PickerRow::Stickers(row) if row.len() == COLUMNS));
        assert!(matches!(&layout.rows[2], PickerRow::Stickers(row) if row.len() == 1));
        assert_eq!(layout.section_at(2), Some(0));
    }

    #[test]
    fn search_matches_names_and_tags_and_drops_recents() {
        let sections = [
            section(SectionKind::Recent, vec![sticker(1, "Wave", "")]),
            section(
                SectionKind::Guild(Id::new(1)),
                vec![sticker(2, "Wave", ""), sticker(3, "Nod", "yes, 👍")],
            ),
            section(SectionKind::Pack(Id::new(2)), vec![sticker(4, "Cry", "")]),
        ];
        let layout = picker_rows(sections, " 👍 ");
        assert_eq!(layout.sections.len(), 1);
        assert!(
            matches!(&layout.rows[1], PickerRow::Stickers(row) if row[0].item.id == Id::new(3))
        );
        assert!(picker_rows([], "wave").rows.is_empty());
    }
}
