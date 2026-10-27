//! The main pane to the right of the sidebar: a conversation's header, its
//! message list, and the composer.

mod channel;
mod dm;
mod message_list;

use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    input::Input, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::{HomeScreen, View};
use crate::ui::button::Button;
use crate::ui::depth::radius;
use crate::ui::window_controls::WindowControls;

const HEADER_HEIGHT: f32 = 48.;

impl HomeScreen {
    pub(in crate::screens::home) fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        if self.view == View::DirectMessages {
            return self.render_dm_content(cx);
        }

        if self.loading {
            return pane(message_list::skeleton().into_any_element(), cx);
        }

        if let Some(error) = &self.error {
            return pane(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .child(div().text_color(cx.theme().danger).child(error.clone()))
                    .into_any_element(),
                cx,
            );
        }

        self.render_channel_content(cx)
    }
}

/// The bar across the top of the pane, standing in for the missing system title bar: the name
/// at left, window controls at right, and the space between drags the window.
///
/// The drag region is a *sibling* of the controls, not their parent: window-control hitboxes
/// resolve in paint order, so a wrapping region would be found first and swallow their clicks.
pub(super) fn header(content: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(content)
        .child(
            WindowControls::default()
                .corner(radius::CARD - px(1.))
                // Out through the panel's ring and inset, to the window's corner.
                .reach(px(super::INSET + 1.)),
        )
}

/// The left half of [`header`]. May be empty; it still has to exist to drag the window by.
pub(super) fn header_content() -> Stateful<Div> {
    h_flex()
        .id("conversation-header")
        .flex_1()
        .min_w_0()
        .h_full()
        .px_4()
        .gap_2()
        .items_center()
        // Windows resolves this to the caption, which brings dragging, snapping
        // and double-click-to-maximize with it.
        .when(cfg!(target_os = "windows"), |this| {
            this.window_control_area(WindowControlArea::Drag)
        })
}

/// The buttons at the right of [`header_content`].
///
/// They sit inside the drag region, and Windows asks which window-control area is under the
/// pointer before gpui sees a click: any drag hitbox answers "caption" and the click becomes a
/// window drag. Occluding the row keeps the region out of that hit test.
pub(super) fn header_actions() -> Div {
    h_flex().flex_shrink_0().gap_2().items_center().occlude()
}

const SEARCH_WIDTH: f32 = 200.;

impl HomeScreen {
    /// Toggles pinned messages, anchored to the header's bottom rather than the click.
    pub(super) fn render_pins_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let open = self.pins_popup.is_some();
        Button::new("toggle-pins")
            .icon(Icon::default().path("icons/pin.svg"))
            .ghost()
            .small()
            .selected(open)
            .tooltip("Pinned Messages")
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                let below_header = px(super::INSET + HEADER_HEIGHT + 4.);
                let position = point(event.position().x + px(16.), below_header);
                this.toggle_pins(position, cx);
            }))
    }

    /// Only drawn for now: typing in it searches nothing.
    pub(super) fn render_search_box(&self, cx: &App) -> impl IntoElement {
        div().w(px(SEARCH_WIDTH)).flex_shrink_0().child(
            Input::new(&self.search_input)
                .small()
                .focus_ring(false)
                .suffix(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                ),
        )
    }
}

/// A pane whose header names nothing (loading, error, no selection). The window controls
/// live in that header, so it is drawn regardless.
pub(super) fn pane(body: AnyElement, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .child(header(header_content(), cx))
        .child(body)
        .into_any_element()
}
