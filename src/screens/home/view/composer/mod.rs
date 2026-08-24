//! The message composer: the reply banner, the staged-attachment tray, and the
//! text input, wrapped in one rounded surface.

mod attachments;
mod reply_banner;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, Size, StyleSized as _, button::Button,
    button::ButtonVariants as _, h_flex, input::Input, v_flex,
};

use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// `can_send` is false on channels the user lacks `SEND_MESSAGES` on, where
    /// a notice takes the composer's place.
    pub(super) fn render_message_bar(&self, can_send: bool, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let has_attachments = !self.pending_attachments.is_empty();

        if !can_send {
            return v_flex()
                .w_full()
                .flex_shrink_0()
                .px_2()
                .pb_2()
                .gap_1()
                .child(
                    // Same surface and metrics as the composer below, so the
                    // conversation doesn't resize when switching between a
                    // channel the user can post in and one they can't. The
                    // `input_*` helpers are what `Input` sizes itself with.
                    h_flex()
                        .w_full()
                        .rounded(px(8.))
                        .bg(theme.secondary)
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .pl_1()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .items_center()
                                .input_size(Size::Medium)
                                .input_text_size(Size::Medium)
                                .text_color(theme.muted_foreground)
                                .child(
                                    "You do not have permission to send messages in this channel.",
                                ),
                        ),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .flex_shrink_0()
            .px_2()
            .pb_2()
            .gap_1()
            .when_some(self.send_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                // Wrap the composer in our own rounded surface so the reply
                // banner, attachment tray, and input read as one control. The
                // input's own border and bright focus ring are switched off in
                // favour of this.
                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .bg(theme.secondary)
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
                    .when_some(self.replying_to.clone(), |this, target| {
                        this.child(self.render_reply_banner(&target, cx))
                    })
                    .when(has_attachments, |this| {
                        this.child(self.render_attachment_previews(cx))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .pl_1()
                            .child(
                                Button::new("add-attachment")
                                    .icon(IconName::Plus)
                                    .ghost()
                                    .small()
                                    .flex_shrink_0()
                                    .tooltip("Add attachment")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pick_attachments(window, cx);
                                    })),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.message_input)
                                        .appearance(false)
                                        .focus_bordered(false),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
