//! The GIF picker: browsing and searching Tenor, the user's favorite GIFs, and
//! sending one. Favorites live in Discord's settings, so they follow the account.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use crate::discord::{self, FavoriteGif, FavoriteGifs};
use crate::screens::home::gifs::{
    CategoryTarget, CategoryTile, GifPage, GifRow, GifTile, category_rows, justify,
};
use crate::screens::home::{GifPicker, HomeScreen};
use crate::ui::smooth_scroll::SmoothScroll;

use super::gif::release_all;

/// Typing pauses this long before Tenor is asked.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Settings-proto is ratelimited heavily, so a run of stars is saved as one.
const FAVORITES_FLUSH_DELAY: Duration = Duration::from_secs(2);

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_gif_picker(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.gif_picker.is_some() {
            return self.close_gif_picker(window, cx);
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search Tenor"));
        let search_changed = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.gif_search_changed(window, cx);
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        let list = ListState::new(0, ListAlignment::Top, px(200.));
        self.gif_picker = Some(GifPicker {
            position,
            search,
            page: GifPage::Home,
            results: None,
            searching: None,
            rows: Rc::default(),
            scroll: SmoothScroll::list(list.clone()),
            list,
            clips: Default::default(),
            images: RetainAllImageCache::new(cx),
            _search_changed: search_changed,
        });
        self.refresh_gif_picker(cx);
        self.load_gif_home(cx);
    }

    pub(in crate::screens::home) fn close_gif_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.gif_picker.take() else {
            return;
        };
        release_all(&picker.clips, window, cx);
        picker
            .images
            .update(cx, |images, cx| images.clear(window, cx));
        self.message_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Back to the tiles, with the search cleared.
    pub(in crate::screens::home) fn gif_picker_home(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_gif_page(GifPage::Home, "", window, cx);
    }

    pub(in crate::screens::home) fn open_gif_category(
        &mut self,
        target: &CategoryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            CategoryTarget::Favorites => self.open_gif_page(GifPage::Favorites, "", window, cx),
            CategoryTarget::Trending => self.open_gif_page(GifPage::Trending, "", window, cx),
            CategoryTarget::Search(term) => {
                self.open_gif_page(GifPage::Search(term.clone()), term, window, cx);
                self.search_gifs(term.clone(), Duration::ZERO, cx);
            }
        }
    }

    fn open_gif_page(
        &mut self,
        page: GifPage,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = &mut self.gif_picker else {
            return;
        };
        let placeholder = match page {
            GifPage::Favorites => "Search your favorite GIFs",
            _ => "Search Tenor",
        };
        picker.page = page;
        picker.searching = None;
        // Neither emits a change, so the page set above stands.
        picker.search.update(cx, |search, cx| {
            search.set_value(query.to_owned(), window, cx);
            search.set_placeholder(placeholder, window, cx);
            search.focus(window, cx);
        });
        self.refresh_gif_picker(cx);
    }

    fn gif_search_changed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.gif_picker else {
            return;
        };
        let query = picker.search.read(cx).value().trim().to_owned();
        match &picker.page {
            GifPage::Favorites => {}
            _ if query.is_empty() => {
                picker.page = GifPage::Home;
                picker.searching = None;
            }
            _ => {
                picker.page = GifPage::Search(query.clone());
                self.search_gifs(query, SEARCH_DEBOUNCE, cx);
            }
        }
        self.refresh_gif_picker(cx);
    }

    fn search_gifs(&mut self, query: String, debounce: Duration, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.gif_picker else {
            return;
        };
        if picker
            .results
            .as_ref()
            .is_some_and(|(searched, _)| *searched == query)
        {
            picker.searching = None;
            return;
        }
        picker.searching = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(debounce).await;
            let result = discord::search_gifs(query.clone()).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(picker) = &mut this.gif_picker {
                    picker.searching = None;
                    picker.results = Some((query, result));
                }
                this.refresh_gif_picker(cx);
            });
        }));
    }

    fn load_gif_home(&mut self, cx: &mut Context<Self>) {
        let home = &mut self.gif_home;
        if !home.categories.is_empty() || home.loading {
            return;
        }
        home.loading = true;
        home.error = None;
        cx.spawn(async move |this, cx| {
            let (categories, trending) =
                futures::join!(discord::fetch_gif_categories(), discord::trending_gifs());
            let _ = this.update(cx, |this, cx| {
                let home = &mut this.gif_home;
                home.loading = false;
                match (categories, trending) {
                    (Ok(categories), Ok(trending)) => {
                        home.categories = categories;
                        home.trending = trending;
                    }
                    (Err(err), _) | (_, Err(err)) => home.error = Some(err),
                }
                this.refresh_gif_picker(cx);
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn refresh_gif_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &self.gif_picker else {
            return;
        };
        let query = picker.search.read(cx).value().trim().to_owned();
        let rows = match &picker.page {
            GifPage::Home => {
                let favorites = CategoryTile {
                    target: CategoryTarget::Favorites,
                    name: "Favorites".into(),
                    image: None,
                };
                let trending = CategoryTile {
                    target: CategoryTarget::Trending,
                    name: "Trending GIFs".into(),
                    image: self.gif_home.trending.first().map(|gif| gif.poster.clone()),
                };
                let tiles = [favorites, trending]
                    .into_iter()
                    .chain(self.gif_home.categories.iter().map(CategoryTile::from))
                    .collect();
                category_rows(tiles)
            }
            GifPage::Trending => {
                justify(self.gif_home.trending.iter().map(GifTile::from).collect())
            }
            GifPage::Favorites => justify(
                self.favorite_gifs
                    .gifs
                    .sorted()
                    .iter()
                    .filter(|gif| gif.matches(&query))
                    .map(GifTile::from)
                    .collect(),
            ),
            GifPage::Search(searched) => match &picker.results {
                Some((query, Ok(gifs))) if query == searched => {
                    justify(gifs.iter().map(GifTile::from).collect())
                }
                _ => Vec::new(),
            },
        };
        let Some(picker) = &mut self.gif_picker else {
            return;
        };
        // A star toggled in place shouldn't send the list back to the top.
        let same_shape = picker.rows.len() == rows.len()
            && picker
                .rows
                .iter()
                .zip(&rows)
                .all(|(old, new)| same_row(old, new));
        picker.rows = Rc::new(rows);
        if !same_shape {
            picker.list.reset(picker.rows.len());
            // `reset` leaves the list at its top. A seek there would land at the bottom: the
            // setter goes by measured height, and a just-reset list's rows all measure zero.
            picker.scroll.sync();
        }
        cx.notify();
    }

    /// The search's error, when the page it's on failed to load.
    pub(in crate::screens::home) fn gif_picker_error(&self) -> Option<&str> {
        let picker = self.gif_picker.as_ref()?;
        match &picker.page {
            GifPage::Home | GifPage::Trending => self.gif_home.error.as_deref(),
            GifPage::Search(searched) => match &picker.results {
                Some((query, Err(err))) if query == searched => Some(err),
                _ => None,
            },
            GifPage::Favorites => None,
        }
    }

    pub(in crate::screens::home) fn gif_picker_loading(&self) -> bool {
        let Some(picker) = self.gif_picker.as_ref() else {
            return false;
        };
        match &picker.page {
            GifPage::Home | GifPage::Trending => self.gif_home.loading,
            GifPage::Search(_) => picker.searching.is_some(),
            GifPage::Favorites => !self.favorite_gifs.loaded,
        }
    }

    /// Sends the GIF's page link on its own, which Discord unfurls into the GIF, and
    /// leaves what's typed for the next message, as Discord's client does. Shift keeps
    /// the picker open.
    pub(in crate::screens::home) fn pick_gif(
        &mut self,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.modifiers().shift {
            self.close_gif_picker(window, cx);
        }
        self.send_link(url, window, cx);
    }

    pub(in crate::screens::home) fn is_favorite_gif(&self, url: &str) -> bool {
        self.favorite_gifs.gifs.contains(url)
    }

    pub(in crate::screens::home) fn toggle_favorite_gif(
        &mut self,
        gif: FavoriteGif,
        cx: &mut Context<Self>,
    ) {
        if !self.favorite_gifs.loaded {
            return;
        }
        let favorites = &mut self.favorite_gifs.gifs;
        if favorites.contains(&gif.url) {
            favorites.remove(&gif.url);
        } else {
            favorites.add(gif);
        }
        self.schedule_favorites_flush(cx);
        self.refresh_gif_picker(cx);
    }

    pub(in crate::screens::home) fn set_favorite_gifs(
        &mut self,
        favorites: FavoriteGifs,
        cx: &mut Context<Self>,
    ) {
        // A save of ours still waiting would overwrite this with the older list.
        if self.favorite_gifs.flush.is_some() {
            return;
        }
        self.favorite_gifs.gifs = favorites;
        self.favorite_gifs.loaded = true;
        self.refresh_gif_picker(cx);
    }

    fn schedule_favorites_flush(&mut self, cx: &mut Context<Self>) {
        if self.favorite_gifs.flush.is_some() {
            return;
        }
        self.favorite_gifs.flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FAVORITES_FLUSH_DELAY).await;
            let Ok(favorites) = this.update(cx, |this, _| this.favorite_gifs.gifs.clone()) else {
                return;
            };
            let result = discord::save_favorite_gifs(favorites).await;
            let _ = this.update(cx, |this, _| this.favorite_gifs.flush = None);
            if let Err(err) = result {
                log::error!("failed to save favorite GIFs: {err}");
            }
        }));
    }
}

fn same_row(old: &GifRow, new: &GifRow) -> bool {
    match (old, new) {
        (GifRow::Categories(old), GifRow::Categories(new)) => old.len() == new.len(),
        (GifRow::Gifs { tiles: old, .. }, GifRow::Gifs { tiles: new, .. }) => {
            old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.0.url == b.0.url)
        }
        _ => false,
    }
}
