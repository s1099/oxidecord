//! Rendering for [`HomeScreen`], one module per region of the screen.

mod composer;
mod content;
mod markdown;
mod member_list;
mod message;
mod profile;
mod rail;
mod sidebar;
mod user_panel;
mod voice;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex, label::Label, v_flex,
};

use super::{HomeScreen, View};
use crate::discord::GatewayConnectionStatus;
use crate::ui::depth;

const MESSAGE_PADDING_X: f32 = 16.;

/// Vertical gap, in pixels, between two consecutive author groups. It sits
/// outside each message's hover highlight, as in Discord.
const GROUP_GAP: f32 = 14.;

impl Render for HomeScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only a pane that's mid-glide does anything here; the rest just resync.
        for scroll in [
            &mut self.rail_scroll,
            &mut self.sidebar_scroll,
            &mut self.dm_scroll,
            &mut self.messages_scroll,
        ] {
            scroll.step(window);
        }

        // Follows whichever way the selection changed: a channel click, a
        // guild switch, the DM view, a voice join.
        self.sync_member_list();

        // Holding shift lays the message toolbar's hidden actions out inline;
        // modifier changes don't repaint on their own, so the listener below
        // nudges the screen when shift goes down or up.
        self.shift_held = window.modifiers().shift;

        // A watched stream is decoded at most at the window's size, which is
        // as large as the stage can draw it.
        self.release_stream_frames(window);
        if let Some(stream) = self
            .watching
            .as_ref()
            .and_then(|watch| watch.stream.as_ref())
        {
            let size = window.viewport_size();
            let scale = window.scale_factor();
            stream.set_target((
                (f32::from(size.width) * scale) as u32,
                (f32::from(size.height) * scale) as u32,
            ));
        }

        let sidebar = match self.view {
            View::DirectMessages => Some(self.render_dm_sidebar(cx).into_any_element()),
            View::Guild => (self.selected_guild.is_some() || self.loading)
                .then(|| self.render_channel_sidebar(cx).into_any_element()),
        };
        let connection_notice = match self.gateway_status {
            GatewayConnectionStatus::Connecting => Some("Connecting to Discord…"),
            GatewayConnectionStatus::Connected => None,
            GatewayConnectionStatus::Reconnecting => Some("Reconnecting…"),
            GatewayConnectionStatus::Disconnected => Some("Disconnected"),
        };

        h_flex()
            .size_full()
            // Anchors the profile popout's full-screen dismiss layer.
            .relative()
            // The rail and sidebar sit straight on the window; the conversation
            // is a panel lifted off it (see `inset`).
            .bg(cx.theme().sidebar)
            .on_action(cx.listener(Self::on_paste_attachment))
            .on_action(cx.listener(Self::on_edit_last_message))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                if this.shift_held != event.modifiers.shift {
                    cx.notify();
                }
            }))
            // Puts the screen on the focus path, which is the only path
            // modifier events travel. Not on the root: a focusable element
            // prevents the default on every mouse down over it, which Windows
            // reads as the app having handled the click — killing the window
            // controls and the drag region.
            .child(div().track_focus(&self.focus_handle))
            .child(self.render_server_rail(cx))
            .children(sidebar)
            .child(inset(
                v_flex()
                    .size_full()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(self.render_content(cx)),
                    )
                    .when_some(connection_notice, |this, label| {
                        this.child(
                            div()
                                .id("gateway-connection-status")
                                .flex_shrink_0()
                                .px_4()
                                .py_1()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child(Label::new(label)),
                        )
                    })
                    .into_any_element(),
                cx,
            ))
            .children(self.render_profile_popup(cx))
            .children(self.render_emoji_picker(cx))
    }
}

/// Gap between the conversation panel and the window's edges.
const INSET: f32 = 8.;

/// The conversation pane as a panel inset from the window: rounded, ringed,
/// and resting on the sidebar colour, the way the toolkit's inset layout reads.
fn inset(content: AnyElement, cx: &App) -> impl IntoElement {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .py(px(INSET))
        .pr(px(INSET))
        .child(
            depth::card(depth::radius::CARD, cx)
                .size_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(content),
        )
        // The strip above the panel is window edge, so it drags the window the
        // way the header inside the panel does. A sibling of the panel rather
        // than its parent, so it can't swallow the window controls' clicks.
        .when(cfg!(target_os = "windows"), |this| {
            this.child(
                div()
                    .id("inset-drag")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(INSET))
                    .window_control_area(WindowControlArea::Drag),
            )
        })
}

/// An avatar with its picture when there is one, and the name's initials
/// otherwise.
fn avatar(name: String, url: Option<String>, size: Pixels) -> Avatar {
    let avatar = Avatar::new().name(name).with_size(size);
    match url {
        Some(url) => avatar.src(url),
        None => avatar,
    }
}

/// The small bar Discord hangs off the left of a channel, DM, or guild with
/// something unread. Placed against the row's left edge, `inset` out from it
/// into the gutter beside the list.
fn unread_pill(inset: Pixels, cx: &App) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(-inset)
        .flex()
        .items_center()
        .child(
            div()
                .w(px(4.))
                .h(px(8.))
                .rounded_r(px(4.))
                .bg(cx.theme().foreground),
        )
}
