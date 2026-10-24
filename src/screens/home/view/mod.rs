//! Rendering for [`HomeScreen`], one module per region of the screen.

mod composer;
mod content;
mod image_viewer;
mod markdown;
mod member_list;
mod message;
mod profile;
mod rail;
mod sidebar;
mod user_panel;
mod voice;

use gpui_kit::component::{ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{HOME_CONTEXT, HomeScreen, MarkConversationRead, View};
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
        if let Some(picker) = &mut self.emoji_picker {
            picker.scroll.step(window);
        }
        // After the glide's step, so it reads where this frame draws the list.
        self.sync_at_bottom(cx);

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

        let image_viewer = self.render_image_viewer(window, cx);

        h_flex()
            .size_full()
            // Anchors the profile popout's full-screen dismiss layer.
            .relative()
            // The rail and sidebar sit straight on the window; the conversation
            // is a panel lifted off it (see `inset`).
            .bg(cx.theme().sidebar)
            .key_context(HOME_CONTEXT)
            .on_action(cx.listener(Self::on_paste_attachment))
            .on_action(cx.listener(Self::on_edit_last_message))
            .on_action(cx.listener(|this, _: &MarkConversationRead, window, cx| {
                if this.image_viewer.is_some() {
                    this.close_image_viewer(window, cx);
                } else {
                    this.mark_conversation_read(window, cx);
                }
            }))
            .on_action(cx.listener(Self::on_copy_selection))
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
            .child(inset(self.render_content(cx), cx))
            .children(self.render_profile_popup(cx))
            .children(self.render_pins_popup(cx))
            .children(self.render_emoji_picker(cx))
            .children(image_viewer)
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

/// The red count Discord puts on whatever has unread messages pinging the
/// user. gpui-component's `Badge` only sits in a corner, without the ring that
/// cuts it out of an icon, so this is drawn by hand. `None` with no mentions.
fn mention_badge(count: u32, cx: &App) -> Option<Div> {
    if count == 0 {
        return None;
    }
    let theme = cx.theme();
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    Some(
        h_flex()
            .flex_shrink_0()
            .justify_center()
            .min_w(px(16.))
            .h(px(16.))
            .px(px(4.))
            .rounded_full()
            .bg(theme.danger)
            .text_color(theme.danger_foreground)
            .text_size(px(11.))
            .line_height(px(16.))
            .font_weight(FontWeight::BOLD)
            .child(label),
    )
}
