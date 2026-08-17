//! The main pane to the right of the sidebar: a conversation's header, its
//! message list, and the composer.

mod channel;
mod dm;
mod message_list;

use gpui::*;
use gpui_component::{ActiveTheme as _, v_flex};

use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(in crate::screens::home) fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        if self.view == View::DirectMessages {
            return self.render_dm_content(cx);
        }

        if self.loading {
            return message_list::skeleton().into_any_element();
        }

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(cx.theme().danger).child(error.clone()))
                .into_any_element();
        }

        self.render_channel_content(cx)
    }
}
