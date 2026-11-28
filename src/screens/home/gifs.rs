//! The GIF picker's contents. Before anything is typed it shows tiles for the
//! favorites, Tenor's trending GIFs and Tenor's featured searches; GIFs are laid
//! out in justified rows, each scaled to fill the grid's width at a shared height.

use gpui_kit::Task;

use crate::discord::{FavoriteGif, FavoriteGifs, Gif, GifCategory, GifFormat};

/// The grid's inner width, inside the picker's padding.
pub(in crate::screens::home) const GRID_WIDTH: f32 = 424.;
pub(in crate::screens::home) const GAP: f32 = 4.;
/// What a row is drawn at before it's scaled to fill the width.
const ROW_HEIGHT: f32 = 110.;
pub(in crate::screens::home) const CATEGORY_COLUMNS: usize = 2;
pub(in crate::screens::home) const CATEGORY_HEIGHT: f32 = 100.;
/// Narrower or wider than this and a tile would be a sliver or the whole row.
const MIN_ASPECT: f32 = 0.5;
const MAX_ASPECT: f32 = 2.5;

/// Tenor's featured searches and trending GIFs, fetched once a session.
#[derive(Default)]
pub(in crate::screens::home) struct GifHome {
    pub categories: Vec<GifCategory>,
    pub trending: Vec<Gif>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub(in crate::screens::home) struct FavoriteGifState {
    pub gifs: FavoriteGifs,
    /// Saving before Discord's copy has loaded would replace it with only this
    /// session's favorites.
    pub loaded: bool,
    pub flush: Option<Task<()>>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(in crate::screens::home) enum GifPage {
    Home,
    Trending,
    /// The search box narrows the favorites rather than asking Tenor.
    Favorites,
    Search(String),
}

/// A GIF drawn in the grid, from Tenor or a favorite.
#[derive(Clone, Debug)]
pub(in crate::screens::home) struct GifTile {
    /// The page URL: what's sent, and the favorite's key.
    pub url: String,
    /// What the tile animates.
    pub preview: String,
    pub format: GifFormat,
    /// A still under the clip until it loads; favorites keep none.
    pub poster: Option<String>,
    pub width: u32,
    pub height: u32,
    /// What a favorite saves, as Discord's client does: the full-size media.
    pub favorite: FavoriteGif,
}

impl From<&Gif> for GifTile {
    fn from(gif: &Gif) -> Self {
        Self {
            url: gif.url.clone(),
            preview: gif.preview.clone(),
            format: GifFormat::Video,
            poster: Some(gif.poster.clone()),
            width: gif.width,
            height: gif.height,
            favorite: FavoriteGif {
                url: gif.url.clone(),
                src: gif.src.clone(),
                format: GifFormat::Video,
                width: gif.width,
                height: gif.height,
                order: 0,
            },
        }
    }
}

impl From<&FavoriteGif> for GifTile {
    fn from(gif: &FavoriteGif) -> Self {
        Self {
            url: gif.url.clone(),
            preview: gif.src.clone(),
            format: gif.format,
            poster: None,
            width: gif.width,
            height: gif.height,
            favorite: gif.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::screens::home) enum CategoryTarget {
    Favorites,
    Trending,
    Search(String),
}

#[derive(Clone, Debug)]
pub(in crate::screens::home) struct CategoryTile {
    pub target: CategoryTarget,
    pub name: String,
    /// A still behind the name.
    pub image: Option<String>,
}

impl From<&GifCategory> for CategoryTile {
    fn from(category: &GifCategory) -> Self {
        Self {
            target: CategoryTarget::Search(category.term.clone()),
            name: category.name.clone(),
            image: Some(category.image.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::screens::home) enum GifRow {
    Categories(Vec<CategoryTile>),
    /// Each tile with the width it's drawn at.
    Gifs {
        tiles: Vec<(GifTile, f32)>,
        height: f32,
    },
}

pub(in crate::screens::home) fn category_rows(tiles: Vec<CategoryTile>) -> Vec<GifRow> {
    tiles
        .chunks(CATEGORY_COLUMNS)
        .map(|row| GifRow::Categories(row.to_vec()))
        .collect()
}

/// Fills each row until it's at least the grid's width, then shrinks it to fit,
/// so no row is taller than [`ROW_HEIGHT`]. The last is left short rather than
/// stretched.
pub(in crate::screens::home) fn justify(tiles: Vec<GifTile>) -> Vec<GifRow> {
    let mut rows = Vec::new();
    let mut row: Vec<(GifTile, f32)> = Vec::new();
    let mut natural = 0.;
    for tile in tiles {
        let aspect = if tile.width > 0 && tile.height > 0 {
            (tile.width as f32 / tile.height as f32).clamp(MIN_ASPECT, MAX_ASPECT)
        } else {
            1.
        };
        let width = ROW_HEIGHT * aspect;
        natural += width;
        row.push((tile, width));
        if natural + gaps(row.len()) >= GRID_WIDTH {
            rows.push(close_row(std::mem::take(&mut row), natural));
            natural = 0.;
        }
    }
    if !row.is_empty() {
        rows.push(close_row(row, natural));
    }
    rows
}

fn gaps(tiles: usize) -> f32 {
    GAP * tiles.saturating_sub(1) as f32
}

fn close_row(mut row: Vec<(GifTile, f32)>, natural: f32) -> GifRow {
    let scale = ((GRID_WIDTH - gaps(row.len())) / natural).min(1.);
    for (_, width) in &mut row {
        *width = (*width * scale).floor();
    }
    GifRow::Gifs {
        tiles: row,
        height: (ROW_HEIGHT * scale).floor(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(width: u32, height: u32) -> GifTile {
        GifTile::from(&FavoriteGif {
            url: String::new(),
            src: String::new(),
            format: GifFormat::Video,
            width,
            height,
            order: 0,
        })
    }

    #[test]
    fn rows_fit_the_width() {
        let rows = justify((0..10).map(|_| tile(160, 90)).collect());
        for row in &rows[..rows.len() - 1] {
            let GifRow::Gifs { tiles, height } = row else {
                panic!("not a GIF row");
            };
            let used: f32 = tiles.iter().map(|(_, width)| width).sum::<f32>() + gaps(tiles.len());
            assert!(used <= GRID_WIDTH && used > GRID_WIDTH - tiles.len() as f32);
            assert!(*height <= ROW_HEIGHT);
        }
    }

    #[test]
    fn a_short_last_row_keeps_its_height() {
        let rows = justify(vec![tile(100, 100)]);
        let GifRow::Gifs { tiles, height } = &rows[0] else {
            panic!("not a GIF row");
        };
        assert_eq!(*height, ROW_HEIGHT);
        assert_eq!(tiles[0].1, ROW_HEIGHT);
    }
}
