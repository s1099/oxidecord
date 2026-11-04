//! Twemoji art for Unicode emoji, so they look the same as in Discord's client
//! instead of whatever the system font draws.
//!
//! Each image is fetched from the CDN the first time it's shown and kept in the
//! config directory, so the binary carries none of them. Decoded images live in a
//! capped LRU rather than gpui's retain-all cache: scrolling the whole picker
//! would otherwise keep thousands decoded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::*;
use tokio::sync::Semaphore;

use crate::platform::{http, prefs, runtime};

const BASE_URL: &str = "https://cdn.jsdelivr.net/gh/jdecked/twemoji@17.0.3/assets/72x72";

/// A 72px image is about 20 KB decoded, so this stays within a few megabytes
/// while holding several screenfuls.
const CAPACITY: usize = 384;

/// Opening the picker asks for a whole page at once.
const MAX_DOWNLOADS: usize = 6;

/// Long enough that being offline doesn't mean a request per frame.
const RETRY_AFTER: Duration = Duration::from_secs(300);

static DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| prefs::config_file("twemoji"));

struct Twemoji(Entity<TwemojiCache>);

impl Global for Twemoji {}

pub fn init(cx: &mut App) {
    let cache = cx.new(|cx| TwemojiCache {
        this: cx.weak_entity(),
        entries: HashMap::new(),
        tick: 0,
        ready: 0,
    });
    cx.set_global(Twemoji(cache));
}

pub fn cache(cx: &App) -> Entity<TwemojiCache> {
    cx.global::<Twemoji>().0.clone()
}

/// Shows the system font's glyph until the image is on disk, or if it never is.
pub fn glyph(text: &str, size: Pixels, cache: &Entity<TwemojiCache>) -> AnyElement {
    let text: SharedString = text.to_string().into();
    let Some(path) = path(&text) else {
        return system_glyph(text, size);
    };
    let loading = text.clone();
    img(path)
        .image_cache(cache)
        .size(size)
        .flex_shrink_0()
        .with_loading(move || system_glyph(loading.clone(), size))
        .with_fallback(move || system_glyph(text.clone(), size))
        .into_any_element()
}

fn system_glyph(text: SharedString, size: Pixels) -> AnyElement {
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .text_size(size * 0.85)
        .line_height(size)
        .child(text)
        .into_any_element()
}

/// Memoised, since this runs for every emoji on screen every frame.
fn path(text: &str) -> Option<Arc<Path>> {
    static PATHS: LazyLock<Mutex<HashMap<&'static str, Arc<Path>>>> =
        LazyLock::new(Default::default);
    let emoji = emojis::get(text)?.as_str();
    let dir = DIR.as_ref()?;
    let mut paths = PATHS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Some(
        paths
            .entry(emoji)
            .or_insert_with(|| dir.join(format!("{}.png", file_stem(emoji))).into())
            .clone(),
    )
}

/// Twemoji's own naming: lowercase hex code points joined by dashes, with
/// U+FE0F dropped unless the sequence has a zero-width joiner.
fn file_stem(emoji: &str) -> String {
    let joined = emoji.contains('\u{200D}');
    emoji
        .chars()
        .filter(|&c| joined || c != '\u{FE0F}')
        .map(|c| format!("{:x}", c as u32))
        .collect::<Vec<_>>()
        .join("-")
}

pub struct TwemojiCache {
    this: WeakEntity<Self>,
    entries: HashMap<Arc<Path>, Entry>,
    /// Bumped on every load, so the least recently drawn image is the smallest `used`.
    tick: u64,
    ready: usize,
}

enum Entry {
    Ready {
        item: ImageCacheItem,
        used: u64,
    },
    Downloading,
    /// `None` for one the CDN doesn't have, which won't turn up later.
    Missing(Option<Instant>),
}

impl ImageCache for TwemojiCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let Resource::Path(path) = resource else {
            return Some(Err(ImageCacheError::Asset("not a twemoji path".into())));
        };
        self.tick += 1;
        match self.entries.get_mut(path) {
            Some(Entry::Ready { item, used }) => {
                *used = self.tick;
                return item.use_image(window);
            }
            Some(Entry::Downloading) => return None,
            Some(Entry::Missing(retry)) if retry.is_none_or(|at| Instant::now() < at) => {
                return Some(Err(ImageCacheError::Asset("twemoji unavailable".into())));
            }
            _ => {}
        }

        if !path.exists() {
            self.entries.insert(path.clone(), Entry::Downloading);
            self.download(path.clone(), cx);
            return None;
        }
        let item = ImageCacheItem::new(resource, cx);
        let image = item.use_image(window);
        self.entries.insert(
            path.clone(),
            Entry::Ready {
                item,
                used: self.tick,
            },
        );
        self.ready += 1;
        if self.ready > CAPACITY {
            self.evict_oldest(window, cx);
        }
        image
    }
}

impl TwemojiCache {
    fn download(&self, path: Arc<Path>, cx: &mut App) {
        let this = self.this.clone();
        cx.spawn(async move |cx| {
            let result = runtime::run(download(path.clone()))
                .await
                .unwrap_or(Err(Fetch::Failed));
            _ = this.update(cx, |cache, _| match result {
                // The next load finds it on disk.
                Ok(()) => _ = cache.entries.remove(&path),
                Err(Fetch::NotFound) => _ = cache.entries.insert(path, Entry::Missing(None)),
                Err(Fetch::Failed) => {
                    let retry = Instant::now() + RETRY_AFTER;
                    _ = cache.entries.insert(path, Entry::Missing(Some(retry)));
                }
            });
            cx.refresh();
        })
        .detach();
    }

    fn evict_oldest(&mut self, window: &mut Window, cx: &mut App) {
        let oldest = self
            .entries
            .iter()
            .filter_map(|(path, entry)| match entry {
                Entry::Ready { used, .. } => Some((*used, path.clone())),
                _ => None,
            })
            .min_by_key(|(used, _)| *used);
        let Some((_, path)) = oldest else {
            return;
        };
        if let Some(Entry::Ready { item, .. }) = self.entries.remove(&path) {
            self.ready -= 1;
            // Frees its sprite atlas slot too, which dropping the item alone wouldn't.
            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }
}

enum Fetch {
    NotFound,
    Failed,
}

async fn download(path: Arc<Path>) -> Result<(), Fetch> {
    static PERMITS: Semaphore = Semaphore::const_new(MAX_DOWNLOADS);
    let _permit = PERMITS.acquire().await.map_err(|_| Fetch::Failed)?;
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or(Fetch::NotFound)?;
    let mut bytes = fetch(stem).await;
    // A few sequences keep U+FE0F in Unicode's form but not in Twemoji's file name.
    if matches!(bytes, Err(Fetch::NotFound)) && stem.contains("-fe0f") {
        bytes = fetch(&stem.replace("-fe0f", "")).await;
    }
    let bytes = bytes?;

    let dir = path.parent().ok_or(Fetch::Failed)?;
    std::fs::create_dir_all(dir).map_err(|_| Fetch::Failed)?;
    // Renamed into place so a half-written file is never read as the image.
    let partial = path.with_extension("part");
    std::fs::write(&partial, &bytes).map_err(|_| Fetch::Failed)?;
    std::fs::rename(&partial, &path).map_err(|_| Fetch::Failed)
}

async fn fetch(stem: &str) -> Result<bytes::Bytes, Fetch> {
    let response = http::client()
        .get(format!("{BASE_URL}/{stem}.png"))
        .send()
        .await
        .map_err(|_| Fetch::Failed)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(Fetch::NotFound);
    }
    response
        .error_for_status()
        .map_err(|_| Fetch::Failed)?
        .bytes()
        .await
        .map_err(|_| Fetch::Failed)
}

#[cfg(test)]
mod tests {
    use super::file_stem;

    #[test]
    fn names_files_as_twemoji_does() {
        assert_eq!(file_stem("😀"), "1f600");
        assert_eq!(file_stem("❤\u{FE0F}"), "2764");
        assert_eq!(file_stem("#\u{FE0F}\u{20E3}"), "23-20e3");
        assert_eq!(file_stem("🇺🇸"), "1f1fa-1f1f8");
        assert_eq!(file_stem("🏳\u{FE0F}\u{200D}🌈"), "1f3f3-fe0f-200d-1f308");
    }
}
