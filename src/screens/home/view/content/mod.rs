//! The main pane to the right of the sidebar: a conversation's header, its
//! message list, and the composer.

mod channel;
mod dm;
mod message_list;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, input::Input, v_flex,
};

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
        .child(
            WindowControls::default()
                .corner(radius::CARD - px(1.))
                // Out through the panel's 1px ring and the inset around it, to
                // the window's own corner.
                .reach(px(super::INSET + 1.)),
        )
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

/// The buttons at the right of [`header_content`].
///
/// They sit inside the drag region, and Windows asks which window-control
/// area is under the pointer before a click ever reaches gpui — any drag
/// hitbox under it answers "caption", and the click becomes a window drag.
/// Occluding the row keeps the drag region out of that hit test.
pub(super) fn header_actions() -> Div {
    h_flex().flex_shrink_0().gap_2().items_center().occlude()
}

const SEARCH_WIDTH: f32 = 200.;

impl HomeScreen {
    /// Toggles the conversation's pinned messages, which hang just below the
    /// header — anchored to the header's bottom rather than the click, which
    /// could land anywhere on the button.
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

    /// The message search box at the right of a conversation's header.
    /// Only there to be drawn for now: typing in it searches nothing.
    pub(super) fn render_search_box(&self, cx: &App) -> impl IntoElement {
        div().w(px(SEARCH_WIDTH)).flex_shrink_0().child(
            Input::new(&self.search_input).small().suffix(
                Icon::new(IconName::Search)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            ),
        )
    }
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

/// The red pill a server, channel, or DM row carries for its unread
/// mentions, capped the way Discord's is. Hidden rows show nothing; the
/// caller decides when to add it.
///
/// Hand-rolled like the row pill it sits beside (`unread_pill`): the widget
/// `Badge` pins its count to a child's corner, while these rows want it in
/// the flow at their right edge.
pub(super) fn mention_badge(count: u32, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_shrink_0()
        .h(px(16.))
        .min_w(px(16.))
        .px(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.danger)
        .text_color(white())
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .child(if count > MENTION_BADGE_CAP {
            format!("{MENTION_BADGE_CAP}+")
        } else {
            count.to_string()
        })
}

/// Where a mention pill stops counting and reads "99+".
const MENTION_BADGE_CAP: u32 = 99;
