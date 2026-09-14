//! Rendering for [`HomeScreen`], one module per region of the screen.

mod composer;
mod content;
mod message;
mod profile;
mod rail;
mod sidebar;
mod text;
mod user_panel;
mod voice;

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex};

use super::{HomeScreen, View};

/// Horizontal padding, in pixels, on either side of the message list.
const MESSAGE_PADDING_X: f32 = 16.;

/// Vertical gap, in pixels, between two consecutive author groups. Split evenly
/// between the bottom of the group above and the top of the group below so each
/// message's hover highlight extends symmetrically into the gap.
const GROUP_GAP: f32 = 16.;

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

        // Holding shift lays the message toolbar's hidden actions out inline;
        // modifier changes don't repaint on their own, so the listener below
        // nudges the screen when shift goes down or up.
        self.shift_held = window.modifiers().shift;

        let sidebar = match self.view {
            View::DirectMessages => Some(self.render_dm_sidebar(cx).into_any_element()),
            View::Guild => (self.selected_guild.is_some() || self.loading)
                .then(|| self.render_channel_sidebar(cx).into_any_element()),
        };

        h_flex()
            .size_full()
            // Anchors the profile popout's full-screen dismiss layer.
            .relative()
            .bg(cx.theme().background)
            .on_action(cx.listener(Self::on_paste_attachment))
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
            .child(self.render_content(cx))
            .children(self.render_profile_popup(cx))
    }
}
