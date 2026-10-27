//! The emoji picker opened from the composer, laid out as Discord's: a search
//! box over the emoji the user may send here, grouped by guild and category
//! with a rail of those groups down the side, and the hovered one named
//! underneath.

use std::ops::Range;

use gpui_kit::component::input::{Escape, Input};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::emoji::{COLUMNS, PickerEmoji, PickerRow, SectionHeader, SectionKind};
use crate::screens::home::{EmojiTarget, HomeScreen};
use crate::ui::depth::{self, Level, radius};
use crate::ui::tooltip;

/// Each emoji's square and every list line's height, headers included, so the
/// list can be virtualized.
const CELL: f32 = 44.;
const EMOJI_SIZE: f32 = 32.;
const LIST_HEIGHT: f32 = 380.;
const PADDING: f32 = 8.;
const RAIL_WIDTH: f32 = 48.;
const RAIL_ICON: f32 = 32.;

impl HomeScreen {
    /// From a message's toolbar it hangs left of the button, clear of the message
    /// being reacted to.
    pub(in crate::screens::home) fn render_emoji_picker(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.emoji_picker.as_ref()?;
        let theme = cx.theme();
        let (corner, offset) = match picker.target {
            EmojiTarget::Composer => (Anchor::BottomRight, point(px(16.), px(-24.))),
            EmojiTarget::Reaction(_) => (Anchor::TopRight, point(px(-24.), px(-16.))),
        };

        let list = if picker.layout.rows.is_empty() {
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
                picker.layout.rows.len(),
                cx.processor(|this, range: Range<usize>, _, cx| this.render_picker_rows(range, cx)),
            )
            .track_scroll(picker.scroll.uniform_handle())
            .on_scroll_wheel(cx.listener(|this, event, window, _| {
                if let Some(picker) = &mut this.emoji_picker {
                    picker.scroll.absorb(event, window);
                }
            }))
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
                Some(emoji) => this.child(emoji_glyph(emoji, 28., self, false)).child(
                    div()
                        .truncate()
                        .text_base()
                        .font_semibold()
                        .child(emoji.label()),
                ),
                None => this
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Pick an emoji"),
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
                this.close_emoji_picker(window, cx);
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
                    .children(self.render_picker_rail(cx))
                    .child(
                        div()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .px(px(PADDING))
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
                            this.close_emoji_picker(window, cx);
                        }),
                    )
                    .child(
                        anchored()
                            .anchor(corner)
                            .position(picker.position + offset)
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    fn render_picker_rail(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let picker = self.emoji_picker.as_ref()?;
        let theme = cx.theme();
        let layout = picker.layout.clone();
        let top_row = (-f32::from(picker.scroll.offset()) / CELL).round() as usize;
        let current = layout.section_at(top_row);

        Some(
            v_flex()
                .id("emoji-picker-rail")
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
                        .id(("emoji-rail", ix))
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
                        .child(section_icon(section, RAIL_ICON - 8., RAIL_ICON - 4.))
                        .tooltip(tooltip::text(SharedString::from(section.title.clone())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(picker) = &mut this.emoji_picker {
                                picker.scroll.glide_to(px(-(first_row as f32) * CELL));
                                cx.notify();
                            }
                        }))
                }))
                .into_any_element(),
        )
    }

    fn render_picker_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(layout) = self
            .emoji_picker
            .as_ref()
            .map(|picker| picker.layout.clone())
        else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        layout.rows[range.clone()]
            .iter()
            .zip(range)
            .map(|(row, ix)| match row {
                PickerRow::Header(section) => {
                    render_picker_header(&layout.sections[*section], *section, &theme, cx)
                }
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
        theme: &gpui_kit::component::Theme,
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

fn render_picker_header(
    section: &SectionHeader,
    index: usize,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<HomeScreen>,
) -> AnyElement {
    let kind = section.kind;
    let chevron = if section.collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    h_flex()
        .id(("emoji-header", index))
        .h(px(CELL))
        .items_end()
        .pb_1p5()
        .px_1()
        .gap_1p5()
        .text_sm()
        .font_semibold()
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(|this| this.text_color(theme.foreground))
        .child(section_icon(section, 16., 16.))
        .child(div().truncate().child(section.title.clone()))
        .child(Icon::new(chevron).size(px(14.)).flex_shrink_0())
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_picker_section(kind, cx)))
        .into_any_element()
}

fn section_icon(section: &SectionHeader, icon: f32, avatar: f32) -> AnyElement {
    use emojis::Group;
    let path = match section.kind {
        SectionKind::Guild(_) => {
            return super::super::avatar(section.title.clone(), section.icon.clone(), px(avatar))
                .rounded(px(avatar / 3.))
                .flex_shrink_0()
                .into_any_element();
        }
        SectionKind::Recent => "icons/clock.svg",
        SectionKind::Unicode(group) => match group {
            Group::SmileysAndEmotion => "icons/smile.svg",
            Group::PeopleAndBody => "icons/user.svg",
            Group::AnimalsAndNature => "icons/leaf.svg",
            Group::FoodAndDrink => "icons/coffee.svg",
            Group::TravelAndPlaces => "icons/plane.svg",
            Group::Activities => "icons/trophy.svg",
            Group::Objects => "icons/lightbulb.svg",
            Group::Symbols => "icons/heart.svg",
            Group::Flags => "icons/flag.svg",
        },
    };
    Icon::default()
        .path(path)
        .size(px(icon))
        .flex_shrink_0()
        .into_any_element()
}

/// A locked emoji is drawn greyed out.
pub(super) fn emoji_glyph(
    emoji: &PickerEmoji,
    size: f32,
    screen: &HomeScreen,
    locked: bool,
) -> AnyElement {
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
