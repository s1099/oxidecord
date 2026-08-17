//! The direct-message conversation: its header, messages, and composer.

use gpui::*;
use gpui_component::{ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex, v_flex};

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    pub(super) fn render_dm_content(&self, cx: &Context<Self>) -> AnyElement {
        let Some(dm) = self.selected_dm_info().cloned() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("Select a conversation to start chatting.")
                .into_any_element();
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_dm_header(&dm, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(cx))
            .into_any_element()
    }

    fn render_dm_header(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let mut avatar = Avatar::new().name(dm.name.clone()).with_size(px(28.));
        if let Some(url) = dm.avatar_url.clone() {
            avatar = avatar.src(url);
        }

        h_flex()
            .h(px(48.))
            .w_full()
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(avatar)
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(dm.name.clone()),
            )
    }
}
