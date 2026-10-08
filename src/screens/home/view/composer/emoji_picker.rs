//! The emoji picker opened from the composer: a search box, the emoji the user
//! may send here grouped by guild and category, and the hovered one named
//! underneath.

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::{Escape, Input};
use gpui_component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};

use crate::screens::home::HomeScreen;
use crate::screens::home::emoji::{COLUMNS, PickerEmoji, PickerRow};
use crate::ui::depth::{self, Level, radius};
use crate::ui::tooltip;

/// Each emoji's square, and every list line's height — headers included, so
/// the list can be virtualized.
const CELL: f32 = 40.;
const EMOJI_SIZE: f32 = 30.;
const LIST_HEIGHT: f32 = 360.;
const PADDING: f32 = 8.;

impl HomeScreen {
    /// The open picker, over the whole app like the profile card: a layer that
    /// closes it on a click outside, with the picker standing just above where
    /// its button was clicked.
    pub(in crate::screens::home) fn render_emoji_picker(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.emoji_picker.as_ref()?;
        let theme = cx.theme();

        let list = if picker.rows.is_empty() {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("No emoji match that search.")
                .into_any_element()
        } else {
            uniform_list(
                "emoji-picker-list",
                picker.rows.len(),
                cx.processor(|this, range: Range<usize>, _, cx| this.render_picker_rows(range, cx)),
            )
            .track_scroll(picker.scroll.clone())
            .size_full()
            .into_any_element()
        };

        let footer = h_flex()
            .h(px(44.))
            .px_3()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.4))
            .text_sm()
            .map(|this| match &picker.hovered {
                Some(emoji) => this
                    .child(emoji_glyph(emoji, 24., self, false))
                    .child(div().truncate().child(emoji.label())),
                None => this
                    .text_color(theme.muted_foreground)
                    .child("Pick an emoji"),
            });

        let card = v_flex()
            .occlude()
            .w(px(CELL * COLUMNS as f32 + PADDING * 2.))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Escape in the search box that it doesn't use itself.
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                this.close_emoji_picker(window, cx);
            }))
            .child(
                div()
                    .p(px(PADDING))
                    .child(Input::new(&picker.search).small().cleanable(true)),
            )
            .child(
                div()
                    .h(px(LIST_HEIGHT))
                    .px(px(PADDING))
                    .pb(px(PADDING))
                    .child(list),
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
                            // Consumed, so a click on the picker's own button
                            // closes it instead of closing and reopening it.
                            cx.stop_propagation();
                            this.close_emoji_picker(window, cx);
                        }),
                    )
                    .child(
                        anchored()
                            .anchor(Corner::BottomRight)
                            .position(picker.position + point(px(16.), px(-24.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_picker_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(rows) = self.emoji_picker.as_ref().map(|picker| picker.rows.clone()) else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        rows[range.clone()]
            .iter()
            .zip(range)
            .map(|(row, ix)| match row {
                PickerRow::Header(title) => div()
                    .h(px(CELL))
                    .flex()
                    .items_end()
                    .pb_1()
                    .px_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(title.to_uppercase())
                    .into_any_element(),
                PickerRow::Emojis(emojis) => h_flex()
                    .h(px(CELL))
                    .children(emojis.iter().enumerate().map(|(column, emoji)| {
                        self.render_picker_cell(emoji, ix * COLUMNS + column, &theme, cx)
                    }))
                    .into_any_element(),
            })
            .collect()
    }

    fn render_picker_cell(
        &self,
        emoji: &PickerEmoji,
        index: usize,
        theme: &gpui_component::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let locked = match emoji {
            PickerEmoji::Custom { locked, .. } => *locked,
            PickerEmoji::Unicode(_) => None,
        };
        let hover_emoji = emoji.clone();
        let pick_emoji = emoji.clone();

        div()
            .id(("emoji-cell", index))
            .size(px(CELL))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::ITEM)
            .child(emoji_glyph(emoji, EMOJI_SIZE, self, locked.is_some()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover_emoji(Some(hover_emoji.clone()), cx);
                }
            }))
            .map(|this| match locked {
                Some(reason) => this.cursor_not_allowed().tooltip(tooltip::text(reason)),
                None => this
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.accent))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_emoji(&pick_emoji, window, cx);
                    })),
            })
            .into_any_element()
    }
}

/// The emoji itself at `size`: a glyph for unicode, the CDN image for custom.
/// A locked one is drawn greyed out.
fn emoji_glyph(emoji: &PickerEmoji, size: f32, screen: &HomeScreen, locked: bool) -> AnyElement {
    match emoji {
        PickerEmoji::Unicode(emoji) => div()
            .text_size(px(size * 0.85))
            .line_height(px(size))
            .child(emoji.as_str())
            .into_any_element(),
        PickerEmoji::Custom { emoji, .. } => img(emoji.url())
            .image_cache(&screen.image_cache)
            .size(px(size))
            .object_fit(ObjectFit::Contain)
            .when(locked, |this| this.grayscale(true).opacity(0.4))
            .into_any_element(),
    }
}
