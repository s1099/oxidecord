//! The sticker picker opened from the composer, laid out like the emoji picker:
//! a search box over the stickers the user may send here, grouped by guild and
//! pack with a rail of those groups down the side, and the hovered one named
//! underneath. Only the hovered sticker moves.

use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::Sticker;
use crate::screens::home::HomeScreen;
use crate::screens::home::stickers::{COLUMNS, PickerRow, SectionHeader, SectionIcon};
use crate::screens::home::view::message::Motion;
use crate::ui::depth::{self, Level, radius};
use crate::ui::tooltip;

const CELL: f32 = 96.;
const STICKER_SIZE: f32 = 80.;
const HEADER_HEIGHT: f32 = 36.;
const LIST_HEIGHT: f32 = 400.;
const PADDING: f32 = 8.;
const RAIL_WIDTH: f32 = 48.;
const RAIL_ICON: f32 = 32.;

impl HomeScreen {
    pub(in crate::screens::home) fn render_sticker_picker(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.sticker_picker.as_ref()?;
        let theme = cx.theme();

        let list = if picker.layout.rows.is_empty() {
            let message = if self.sticker_packs.loading {
                "Loading stickers…".to_string()
            } else if let Some(error) = &self.sticker_packs.error {
                format!("Couldn't load Discord's stickers: {error}")
            } else if picker.search.read(cx).value().trim().is_empty() {
                "No stickers you can send here.".to_string()
            } else {
                "No stickers match that search.".to_string()
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
                cx.processor(|this, ix: usize, _, cx| this.render_sticker_row(ix, cx)),
            )
            .size_full()
            .into_any_element()
        };

        let footer = h_flex()
            .h(px(48.))
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.4))
            .map(|this| match &picker.hovered {
                Some(sticker) => this
                    .child(self.sticker_image(
                        &sticker.item,
                        28.,
                        Motion::Still,
                        "sticker-picker-hovered",
                        cx,
                    ))
                    .child(
                        div()
                            .truncate()
                            .text_base()
                            .font_semibold()
                            .child(sticker.item.name.clone()),
                    ),
                None => this
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Pick a sticker to send it"),
            });

        let card = v_flex()
            .occlude()
            .w(px(RAIL_WIDTH + CELL * COLUMNS as f32 + PADDING * 2.))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                this.close_sticker_picker(window, cx);
            }))
            .child(
                div().p_3().border_b_1().border_color(theme.border).child(
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
            )
            .child(
                h_flex()
                    .h(px(LIST_HEIGHT))
                    .items_start()
                    .children(self.render_sticker_rail(cx))
                    .child(
                        div()
                            .id("sticker-picker-list")
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .px(px(PADDING))
                            // An ancestor's, so it runs after the list's own handler.
                            .on_scroll_wheel(cx.listener(|this, event, window, _| {
                                if let Some(picker) = &mut this.sticker_picker {
                                    picker.scroll.absorb(event, window);
                                }
                            }))
                            .child(list),
                    ),
            )
            .child(footer);

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
                            this.close_sticker_picker(window, cx);
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

    fn render_sticker_rail(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.sticker_picker.as_ref()?;
        let theme = cx.theme();
        let layout = picker.layout.clone();
        let current = layout.section_at(picker.list.logical_scroll_top().item_ix);

        Some(
            v_flex()
                .id("sticker-picker-rail")
                .w(px(RAIL_WIDTH))
                .h_full()
                .flex_shrink_0()
                .items_center()
                .py_2()
                .gap_1()
                .bg(theme.sidebar)
                .overflow_y_scroll()
                .track_scroll(&picker.rail_scroll)
                .children(layout.sections.iter().enumerate().map(|(ix, section)| {
                    let active = current == Some(ix);
                    let first_row = section.first_row;
                    div()
                        .id(("sticker-rail", ix))
                        .size(px(RAIL_ICON + 4.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(radius::ITEM)
                        .cursor_pointer()
                        .text_color(if active {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        })
                        .when(active, |this| this.bg(theme.accent))
                        .hover(|this| this.bg(theme.accent).text_color(theme.foreground))
                        .child(self.section_icon(section, ix, RAIL_ICON - 8., RAIL_ICON - 4., cx))
                        .tooltip(tooltip::text(SharedString::from(section.title.clone())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(picker) = &mut this.sticker_picker {
                                picker.list.scroll_to(ListOffset {
                                    item_ix: first_row,
                                    offset_in_item: px(0.),
                                });
                                cx.notify();
                            }
                        }))
                }))
                .into_any_element(),
        )
    }

    fn render_sticker_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(picker) = self.sticker_picker.as_ref() else {
            return div().into_any_element();
        };
        let layout = picker.layout.clone();
        let hovered = picker.hovered.as_ref().map(|sticker| sticker.item.id);
        let theme = cx.theme().clone();
        match &layout.rows[ix] {
            PickerRow::Header(section) => {
                let header = &layout.sections[*section];
                h_flex()
                    .h(px(HEADER_HEIGHT))
                    .items_end()
                    .pb_1p5()
                    .px_1()
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap_1p5()
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .child(self.section_icon(header, *section, 16., 16., cx))
                            .child(div().truncate().child(header.title.clone())),
                    )
                    .into_any_element()
            }
            PickerRow::Stickers { stickers, locked } => h_flex()
                .h(px(CELL))
                .children(stickers.iter().enumerate().map(|(column, sticker)| {
                    let motion = if hovered == Some(sticker.item.id) && locked.is_none() {
                        Motion::FromStill
                    } else {
                        Motion::Still
                    };
                    let index = ix * COLUMNS + column;
                    self.render_sticker_cell(sticker, index, motion, *locked, &theme, cx)
                }))
                .into_any_element(),
        }
    }

    fn render_sticker_cell(
        &self,
        sticker: &Sticker,
        index: usize,
        motion: Motion,
        locked: Option<&'static str>,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover_sticker = sticker.clone();
        let pick_sticker = sticker.clone();
        let id = sticker.item.id;
        div()
            .id(("sticker-cell", index))
            .size(px(CELL))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::ITEM)
            .child(self.sticker_image(
                &sticker.item,
                STICKER_SIZE,
                motion,
                ("sticker-cell-image", index),
                cx,
            ))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover_sticker(Some(hover_sticker.clone()), cx);
                } else if this
                    .sticker_picker
                    .as_ref()
                    .is_some_and(|picker| picker.hovered.as_ref().map(|s| s.item.id) == Some(id))
                {
                    this.hover_sticker(None, cx);
                }
            }))
            .map(|this| match locked {
                Some(reason) => this
                    .cursor_not_allowed()
                    .opacity(0.4)
                    .tooltip(tooltip::text(reason)),
                None => this
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_sticker(&pick_sticker, window, cx);
                    })),
            })
            .into_any_element()
    }

    /// `index` keeps one section's icon apart from another's that shows the same sticker.
    fn section_icon(
        &self,
        section: &SectionHeader,
        index: usize,
        icon: f32,
        avatar: f32,
        cx: &App,
    ) -> AnyElement {
        match &section.icon {
            SectionIcon::Recent => Icon::default()
                .path("icons/clock.svg")
                .size(px(icon))
                .flex_shrink_0()
                .into_any_element(),
            SectionIcon::Guild(url) => {
                super::super::avatar(section.title.clone(), url.clone(), px(avatar))
                    .rounded(px(avatar / 3.))
                    .flex_shrink_0()
                    .into_any_element()
            }
            SectionIcon::Sticker(sticker) => div()
                .flex_shrink_0()
                .child(self.sticker_image(
                    sticker,
                    avatar,
                    Motion::Still,
                    SharedString::from(format!("sticker-section-{index}-{avatar}")),
                    cx,
                ))
                .into_any_element(),
        }
    }
}
