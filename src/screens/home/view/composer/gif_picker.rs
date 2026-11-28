//! The GIF picker opened from the composer: a search box over tiles for the
//! favorites, trending GIFs and Tenor's featured searches, or over the GIFs one of
//! them leads to. Every GIF on screen loops; a star on each saves it as a favorite.

use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::GifFormat;
use crate::screens::home::HomeScreen;
use crate::screens::home::data::gif::{ClipKind, gif_source};
use crate::screens::home::gifs::{
    CATEGORY_COLUMNS, CATEGORY_HEIGHT, CategoryTarget, CategoryTile, GAP, GRID_WIDTH, GifPage,
    GifRow, GifTile,
};
use crate::ui::depth::{self, Level, radius};
use crate::ui::tooltip;

const LIST_HEIGHT: f32 = 440.;
const PADDING: f32 = 8.;
const STAR_SIZE: f32 = 28.;

impl HomeScreen {
    pub(in crate::screens::home) fn render_gif_picker(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.gif_picker.as_ref()?;
        let theme = cx.theme();

        let body = if picker.rows.is_empty() {
            let message = if let Some(error) = self.gif_picker_error() {
                format!("Couldn't load GIFs: {error}")
            } else if self.gif_picker_loading() {
                "Loading GIFs…".to_string()
            } else {
                match &picker.page {
                    GifPage::Favorites if picker.search.read(cx).value().trim().is_empty() => {
                        "No favorites yet. Star a GIF to save it here.".to_string()
                    }
                    GifPage::Favorites => "No favorites match that search.".to_string(),
                    _ => "No GIFs match that search.".to_string(),
                }
            };
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(message)
                .into_any_element()
        } else {
            list(
                picker.list.clone(),
                cx.processor(|this, ix: usize, _, cx| this.render_gif_row(ix, cx)),
            )
            .size_full()
            .into_any_element()
        };

        let title = match &picker.page {
            GifPage::Home | GifPage::Search(_) => None,
            GifPage::Trending => Some("Trending GIFs"),
            GifPage::Favorites => Some("Favorites"),
        };
        let header = h_flex()
            .p_3()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .when(picker.page != GifPage::Home, |this| {
                this.child(
                    crate::ui::button::Button::new("gif-picker-back")
                        .icon(Icon::default().path("icons/arrow-left.svg"))
                        .ghost()
                        .small()
                        .tooltip("Back")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.gif_picker_home(window, cx);
                        })),
                )
            })
            .when_some(title, |this, title| {
                this.child(div().flex_shrink_0().text_sm().font_semibold().child(title))
            })
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&picker.search)
                        .prefix(
                            Icon::new(IconName::Search)
                                .size(px(16.))
                                .text_color(theme.muted_foreground),
                        )
                        .cleanable(true)
                        // Always focused while open, so a ring says nothing.
                        .focus_bordered(false),
                ),
            );

        let card = v_flex()
            .occlude()
            .w(px(GRID_WIDTH + PADDING * 2.))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                this.close_gif_picker(window, cx);
            }))
            .child(header)
            .child(
                div()
                    .id("gif-picker-list")
                    .h(px(LIST_HEIGHT))
                    .w_full()
                    .px(px(PADDING))
                    .pt(px(PADDING))
                    // An ancestor's, so it runs after the list's own handler.
                    .on_scroll_wheel(cx.listener(|this, event, window, _| {
                        if let Some(picker) = &mut this.gif_picker {
                            picker.scroll.absorb(event, window);
                        }
                    }))
                    .child(body),
            )
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Powered by Tenor"),
            );

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            // So a click on the picker's own button closes it rather
                            // than closing and reopening.
                            cx.stop_propagation();
                            this.close_gif_picker(window, cx);
                        }),
                    )
                    .child(
                        anchored()
                            .anchor(Anchor::BottomRight)
                            .position(picker.position + point(px(16.), px(-24.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_gif_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(picker) = self.gif_picker.as_ref() else {
            return div().into_any_element();
        };
        let rows = picker.rows.clone();
        let row = h_flex().gap(px(GAP)).pb(px(GAP));
        match &rows[ix] {
            GifRow::Categories(tiles) => row
                .children(tiles.iter().enumerate().map(|(column, tile)| {
                    self.render_category_tile(tile, ix * CATEGORY_COLUMNS + column, cx)
                }))
                .into_any_element(),
            GifRow::Gifs { tiles, height } => row
                .h(px(*height + GAP))
                .children(tiles.iter().enumerate().map(|(column, (tile, width))| {
                    self.render_gif_tile(tile, (*width, *height), ix * 16 + column, cx)
                }))
                .into_any_element(),
        }
    }

    fn render_category_tile(
        &self,
        tile: &CategoryTile,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picker) = self.gif_picker.as_ref() else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let width = (GRID_WIDTH - GAP * (CATEGORY_COLUMNS - 1) as f32) / CATEGORY_COLUMNS as f32;
        let icon = match tile.target {
            CategoryTarget::Favorites => Some("icons/star.svg"),
            CategoryTarget::Trending => Some("icons/trending-up.svg"),
            CategoryTarget::Search(_) => None,
        };
        let group = SharedString::from(format!("gif-category-{index}"));
        let target = tile.target.clone();
        div()
            .id(("gif-category", index))
            .group(group.clone())
            .relative()
            .w(px(width))
            .h(px(CATEGORY_HEIGHT))
            .flex_shrink_0()
            .rounded(radius::ITEM)
            .overflow_hidden()
            .bg(theme.muted)
            .cursor_pointer()
            .when_some(tile.image.clone(), |this, image| {
                this.child(
                    img(image)
                        .image_cache(&picker.images)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                h_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .bg(black().opacity(0.5))
                    .group_hover(group, |this| this.bg(black().opacity(0.3)))
                    .text_color(white())
                    .font_semibold()
                    .when_some(icon, |this, icon| {
                        this.child(Icon::default().path(icon).size(px(18.)))
                    })
                    .child(div().truncate().child(tile.name.clone())),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_gif_category(&target, window, cx);
            }))
            .into_any_element()
    }

    fn render_gif_tile(
        &self,
        tile: &GifTile,
        (width, height): (f32, f32),
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picker) = self.gif_picker.as_ref() else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let favorite = self.is_favorite_gif(&tile.url);
        let on_favorites = picker.page == GifPage::Favorites;
        let group = SharedString::from(format!("gif-tile-{index}"));
        let clip = match tile.format {
            GifFormat::Video => img(gif_source(
                tile.preview.clone(),
                (width, height),
                ClipKind::Video,
                picker.clips.clone(),
            )),
            GifFormat::Image => img(tile.preview.clone()).image_cache(&picker.images),
        }
        // An id gives gpui somewhere to keep an animated image's frame index.
        .id(("gif-tile-clip", index))
        .absolute()
        .inset_0()
        .size_full()
        .object_fit(ObjectFit::Cover);

        let url = tile.url.clone();
        let saved = tile.favorite.clone();
        let star = div()
            .id(("gif-tile-star", index))
            .absolute()
            .top_1()
            .right_1()
            .size(px(STAR_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::ITEM)
            .bg(black().opacity(0.6))
            .text_color(if favorite { theme.warning } else { white() })
            .hover(|this| this.bg(black().opacity(0.8)))
            // On the favorites page every star is filled, so they only say something on hover.
            .when(!favorite || on_favorites, |this| {
                this.invisible()
                    .group_hover(group.clone(), |style| style.visible())
            })
            .child(
                Icon::default()
                    .path(if favorite {
                        "icons/star-fill.svg"
                    } else {
                        "icons/star.svg"
                    })
                    .size(px(16.)),
            )
            .tooltip(tooltip::text(if favorite {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_favorite_gif(saved.clone(), cx);
            }));

        div()
            .id(("gif-tile", index))
            .group(group)
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_shrink_0()
            .rounded(radius::ITEM)
            .overflow_hidden()
            .bg(theme.muted)
            .cursor_pointer()
            .when_some(tile.poster.clone(), |this, poster| {
                this.child(
                    img(poster)
                        .image_cache(&picker.images)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(clip)
            .child(star)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.pick_gif(url.clone(), window, cx);
            }))
            .into_any_element()
    }
}
