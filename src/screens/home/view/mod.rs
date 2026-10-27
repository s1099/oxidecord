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

use gpui_kit::component::{ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{HOME_CONTEXT, HomeScreen, MarkConversationRead, View};
use crate::ui::depth;

const MESSAGE_PADDING_X: f32 = 16.;

/// Gap in px between author groups, outside each message's hover highlight.
const GROUP_GAP: f32 = 14.;

impl Render for HomeScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        self.sync_member_list();

        // Modifier changes don't repaint on their own, so nudge the screen when shift toggles
        // (it lays the message toolbar's hidden actions out inline).
        self.shift_held = window.modifiers().shift;

        // Decoded at most at the window's size, the largest the stage can draw.
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
            // After the bindings, so only keystrokes no shortcut took.
            .on_key_down(cx.listener(Self::on_type_to_compose))
            // gpui leaves focus where it was when a click lands on nothing focusable; Discord lets
            // go,
            // so typing goes to the composer, not a stale search or edit box. Anything focusable
            // takes mouse down first by preventing default.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !window.default_prevented() {
                        this.focus_handle.focus(window, cx);
                    }
                }),
            )
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                if this.shift_held != event.modifiers.shift {
                    cx.notify();
                }
            }))
            // Puts the screen on the focus path, the only one modifier events travel. Not the root:
            // a focusable element prevents default on every mouse down, which Windows reads as the
            // click handled, killing the window controls and drag region.
            .child(div().track_focus(&self.focus_handle))
            // The account card spans the rail too, so both share a column with it at the bottom.
            .child(
                v_flex()
                    .h_full()
                    .flex_shrink_0()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .child(self.render_server_rail(cx))
                            .children(sidebar),
                    )
                    .child(self.render_sidebar_footer(cx)),
            )
            .child(inset(self.render_content(cx), cx))
            .children(self.render_profile_popup(cx))
            .children(self.render_pins_popup(cx))
            .children(self.render_emoji_picker(cx))
            .children(image_viewer)
    }
}

const INSET: f32 = 8.;

/// The conversation pane as an inset panel: rounded, ringed, on the sidebar colour.
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
        // The strip above the panel is window edge and drags the window like the header.
        // A sibling of the panel so it can't swallow the window controls' clicks.
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

fn avatar(name: String, url: Option<String>, size: Pixels) -> Avatar {
    let avatar = Avatar::new().name(name).with_size(size);
    match url {
        Some(url) => avatar.src(url),
        None => avatar,
    }
}

/// The tag after a bot's name, wherever the name is shown.
fn bot_tag(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_shrink_0()
        .px(px(4.))
        .rounded(px(4.))
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .child("BOT")
}

/// The unread bar Discord hangs off the left of a row, `inset` out from its edge into the gutter.
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

/// The red mention count. gpui-component's `Badge` only sits in a corner, without the ring
/// that cuts it out of an icon, so this is drawn by hand. `None` with no mentions.
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
