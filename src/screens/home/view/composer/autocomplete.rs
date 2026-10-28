//! The suggestions hung above the composer while an `@`, `#` or `:` word is
//! being typed.

use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::screens::home::autocomplete::{Suggestion, Trigger};
use crate::screens::home::channels::channel_icon_path;
use crate::screens::home::view::avatar;
use crate::ui::depth::{self, Level, radius};

use super::emoji_picker::emoji_glyph;

const ROW_HEIGHT: f32 = 34.;

impl HomeScreen {
    /// Covers the typing indicator while open.
    pub(super) fn render_autocomplete(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let autocomplete = self.autocomplete.as_ref()?;
        let theme = cx.theme();

        let (kind, trigger) = match autocomplete.trigger {
            Trigger::User => ("MEMBERS", '@'),
            Trigger::Channel => ("CHANNELS", '#'),
            Trigger::Emoji => ("EMOJI", ':'),
        };
        let title = h_flex()
            .px_2()
            .pt_1()
            .pb(px(6.))
            .gap_1()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(kind)
            .when(!autocomplete.query.is_empty(), |this| {
                this.child("MATCHING").child(
                    div()
                        .truncate()
                        .text_color(theme.foreground)
                        .child(format!("{trigger}{}", autocomplete.query)),
                )
            });
        let rows = autocomplete
            .items
            .iter()
            .enumerate()
            .map(|(index, suggestion)| {
                self.render_suggestion(suggestion, index, index == autocomplete.selected, cx)
            });

        Some(
            v_flex()
                .occlude()
                .absolute()
                .bottom_full()
                .left_2()
                .right_2()
                .mb_1()
                .p_2()
                .bg(theme.sidebar)
                .text_color(theme.sidebar_foreground)
                .border_1()
                .border_color(theme.border)
                .rounded(radius::CONTROL)
                .shadow(depth::shadow(Level::Overlay, cx))
                // Else a click takes focus from the composer.
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .child(title)
                .children(rows)
                .into_any_element(),
        )
    }

    fn render_suggestion(
        &self,
        suggestion: &Suggestion,
        index: usize,
        selected: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (glyph, label, detail): (Option<AnyElement>, String, Option<String>) = match suggestion
        {
            Suggestion::User {
                name,
                username,
                avatar_url,
                ..
            } => (
                Some(avatar(name.clone(), avatar_url.clone(), px(24.)).into_any_element()),
                name.clone(),
                username.clone(),
            ),
            Suggestion::Role { name, color, .. } => (
                Some(
                    div()
                        .text_base()
                        .font_semibold()
                        .text_color(color.map_or(theme.muted_foreground, |color| rgb(color).into()))
                        .child("@")
                        .into_any_element(),
                ),
                name.clone(),
                Some("Role".to_string()),
            ),
            Suggestion::Channel {
                name,
                kind,
                category,
                ..
            } => (
                Some(
                    Icon::default()
                        .path(channel_icon_path(*kind))
                        .text_color(theme.muted_foreground)
                        .into_any_element(),
                ),
                name.clone(),
                category.clone(),
            ),
            Suggestion::Emoji { emoji, guild } => (
                Some(emoji_glyph(emoji, 22., self, false)),
                emoji.label(),
                guild.clone(),
            ),
            Suggestion::Broadcast(broadcast) => (
                None,
                broadcast.text().to_string(),
                Some(broadcast.description().to_string()),
            ),
        };

        h_flex()
            .id(("autocomplete", index))
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.sidebar_accent))
            .children(glyph.map(|glyph| {
                div()
                    .flex()
                    .size(px(24.))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(glyph)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .when(selected, |this| {
                        this.text_color(theme.sidebar_accent_foreground)
                    })
                    .child(label),
            )
            .children(detail.map(|detail| {
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .max_w(relative(0.6))
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(detail)
            }))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover_suggestion(index, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.accept_suggestion(index, window, cx);
            }))
            .into_any_element()
    }
}
