//! Avatars, icons and banners, decoded into one capped LRU shared app-wide.
//!
//! gpui-component's `Avatar` draws a bare `img()`, and an image with no cache of its own
//! inside a `list` row lands in gpui's global asset cache, which never evicts. Every face
//! ever scrolled past would stay decoded, and in the sprite atlas, all session.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::{Sizable as _, avatar::Avatar};
use gpui_kit::*;

/// Roughly a thousand 80px avatars, or a few dozen banners.
const BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// More than are ever on screen at once, so an animated banner eating the budget can't
/// evict what's being drawn and have it fetched again every frame.
const MIN_KEPT: usize = 256;

struct Images(Entity<LruImageCache>);

impl Global for Images {}

pub fn init(cx: &mut App) {
    let cache = cx.new(|_| LruImageCache::new(BUDGET_BYTES));
    cx.set_global(Images(cache));
}

pub fn cache(cx: &App) -> Entity<LruImageCache> {
    cx.global::<Images>().0.clone()
}

pub fn avatar(name: impl Into<SharedString>, url: Option<String>, size: Pixels) -> CachedAvatar {
    let avatar = Avatar::new().name(name).with_size(size);
    CachedAvatar(match url {
        Some(url) => avatar.src(url),
        None => avatar,
    })
}

/// An [`Avatar`] whose image loads through [`cache`]. Styles apply to the avatar itself.
#[derive(IntoElement)]
pub struct CachedAvatar(Avatar);

impl Styled for CachedAvatar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl RenderOnce for CachedAvatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // `Avatar` can't take a cache, so the element around it puts one on the stack for
        // the layout and paint its image loads in.
        image_cache(cache(cx)).flex_none().child(self.0)
    }
}

/// Evicts the least recently drawn images once their decoded size passes a budget,
/// handing their sprite atlas slots back too.
pub struct LruImageCache {
    entries: HashMap<Resource, Entry>,
    budget: usize,
    /// Bumped on every load, so the least recently drawn image has the smallest `used`.
    tick: u64,
}

struct Entry {
    item: ImageCacheItem,
    used: u64,
}

impl LruImageCache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            tick: 0,
        }
    }

    /// Only runs on a miss, so sizes are summed afresh rather than tracked as loads land.
    fn evict(&mut self, window: &mut Window, cx: &mut App) {
        let mut total: usize = self.entries.values().map(Entry::bytes).sum();
        while total > self.budget && self.entries.len() > MIN_KEPT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(resource, _)| resource.clone())
            else {
                return;
            };
            let Some(entry) = self.entries.remove(&oldest) else {
                return;
            };
            total -= entry.bytes();
            // Dropping the item alone would leave its atlas slot allocated.
            if let Some(Ok(image)) = entry.item.get() {
                cx.drop_image(image, Some(window));
            }
        }
    }
}

impl Entry {
    fn bytes(&self) -> usize {
        match self.item.get() {
            Some(Ok(image)) => (0..image.frame_count())
                .map(|frame| {
                    let size = image.size(frame);
                    size.width.0.max(0) as usize * size.height.0.max(0) as usize * 4
                })
                .sum(),
            _ => 0,
        }
    }
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.tick += 1;
        if let Some(entry) = self.entries.get_mut(resource) {
            entry.used = self.tick;
            return entry.item.use_image(window);
        }
        self.evict(window, cx);
        let item = ImageCacheItem::new(resource, cx);
        let image = item.use_image(window);
        self.entries.insert(
            resource.clone(),
            Entry {
                item,
                used: self.tick,
            },
        );
        image
    }
}
