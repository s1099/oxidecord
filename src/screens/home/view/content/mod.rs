//! The main pane to the right of the sidebar: a conversation's header, its
//! message list, and the composer.

mod channel;
mod dm;
mod message_list;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::screens::home::{HomeScreen, View};
use crate::ui::window_controls::WindowControls;

/// Height of the conversation header, in pixels.
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

/// The bar across the top of the conversation pane.
///
/// The window has no system title bar, so this bar stands in for one: whatever
/// names the conversation sits at its left, the window controls at its right,
/// and the space between the two drags the window.
///
/// The draggable region is a *sibling* of the controls rather than their
/// parent. Window-control hitboxes are resolved in paint order, so a region
/// wrapping the buttons would be found first and swallow every click meant for
/// them.
pub(super) fn header(content: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(content)
        .child(WindowControls)
}

/// The left half of [`header`], holding whatever names the conversation. Empty
/// is fine — it still has to be there to drag the window by.
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

/// A conversation pane whose header has nothing to name — while a guild loads,
/// after an error, or with nothing selected. The window controls live in that
/// header, so it is drawn either way.
pub(super) fn pane(body: AnyElement, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .child(header(header_content(), cx))
        .child(body)
        .into_any_element()
}
