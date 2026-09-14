//! The direct-message conversation: its header, messages, and composer.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, v_flex,
};

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;

use super::{header, header_content, pane};

impl HomeScreen {
    pub(super) fn render_dm_content(&self, cx: &Context<Self>) -> AnyElement {
        let Some(dm) = self.selected_dm_info().cloned() else {
            return pane(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a conversation to start chatting.")
                    .into_any_element(),
                cx,
            );
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_dm_header(&dm, cx))
            .children(self.render_dm_stage(cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(true, cx))
            .into_any_element()
    }

    fn render_dm_header(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let mut avatar = Avatar::new().name(dm.name.clone()).with_size(px(28.));
        if let Some(url) = dm.avatar_url.clone() {
            avatar = avatar.src(url);
        }

        // The call buttons drop out while this conversation's call is up —
        // hanging up is the stage's job from there.
        let in_call = self.in_voice_channel(dm.id);

        header(
            header_content()
                .child(avatar)
                .child(
                    div()
                        .flex_shrink_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(dm.name.clone()),
                )
                .child(div().flex_1())
                .when(!in_call, |this| {
                    this.child(
                        Button::new("dm-voice-call")
                            .icon(Icon::default().path("icons/phone.svg"))
                            .ghost()
                            .small()
                            .tooltip("Start Voice Call")
                            .on_click(cx.listener(|this, _, _, cx| this.start_dm_call(false, cx))),
                    )
                    .child(
                        Button::new("dm-video-call")
                            .icon(Icon::default().path("icons/video.svg"))
                            .ghost()
                            .small()
                            .tooltip("Start Video Call")
                            .on_click(cx.listener(|this, _, _, cx| this.start_dm_call(true, cx))),
                    )
                }),
            cx,
        )
    }
}
