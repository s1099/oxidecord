//! The direct-message conversation: its header, messages, and composer.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::dm_avatar;
use crate::ui::button::Button;

use super::{header, header_actions, header_content, pane};

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
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(self.render_messages(cx))
                            .child(self.render_message_bar(true, cx)),
                    )
                    .map(|this| match self.render_search_panel(cx) {
                        Some(panel) => this.child(panel),
                        None => this.children(self.render_dm_member_list(&dm, cx)),
                    }),
            )
            .into_any_element()
    }

    fn render_dm_header(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let avatar = dm_avatar(dm, px(28.), cx.theme().background, None);

        // The call buttons drop out while this conversation's call is up —
        // hanging up is the stage's job from there.
        let in_call = self.in_voice_channel(dm.id);
        let ongoing = !self.voice_participants(dm.id).is_empty();

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
                .child(
                    header_actions()
                        .when(!in_call, |this| {
                            this.child(
                                Button::new("dm-voice-call")
                                    .icon(Icon::default().path("icons/phone.svg"))
                                    .ghost()
                                    .small()
                                    .tooltip(if ongoing {
                                        "Join Voice Call"
                                    } else {
                                        "Start Voice Call"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.start_dm_call(cx))),
                            )
                            .child(
                                Button::new("dm-video-call")
                                    .icon(Icon::default().path("icons/video.svg"))
                                    .ghost()
                                    .small()
                                    .disabled(true)
                                    .tooltip("Video isn't supported yet"),
                            )
                        })
                        .child(self.render_pins_button(cx))
                        .when(dm.recipient_id.is_none(), |this| {
                            let hidden = self.member_list_hidden;
                            this.child(
                                Button::new("toggle-member-list")
                                    .icon(Icon::default().path("icons/users.svg"))
                                    .ghost()
                                    .small()
                                    .selected(!hidden)
                                    .tooltip(if hidden {
                                        "Show Member List"
                                    } else {
                                        "Hide Member List"
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_member_list(cx)),
                                    ),
                            )
                        })
                        .child(self.render_search_box(cx)),
                ),
            cx,
        )
    }
}
