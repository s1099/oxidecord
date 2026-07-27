//! The message composer: the reply banner, the staged-attachment tray, and the
//! text input, wrapped in one rounded surface.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, button::Button, button::ButtonVariants as _,
    h_flex, input::Input, v_flex,
};

use crate::screens::home::data::attachments::{PendingAttachment, format_size};
use crate::screens::home::{HomeScreen, ReplyTarget};

impl HomeScreen {
    pub(super) fn render_message_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let has_attachments = !self.pending_attachments.is_empty();

        v_flex()
            .w_full()
            .flex_shrink_0()
            .px_2()
            .pt_1()
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
    }

    /// The "Replying to <author>" strip that sits atop the composer while a
    /// reply is pending.
    fn render_reply_banner(&self, target: &ReplyTarget, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        h_flex()
            .w_full()
            .h(px(32.))
            .px_3()
            .items_center()
            .justify_between()
            .bg(theme.muted.opacity(0.5))
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Replying to")
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(target.author_name.clone()),
                    ),
            )
            .child(
                Button::new("cancel-reply")
                    .icon(IconName::CircleX)
                    .ghost()
                    .xsmall()
                    .tooltip("Cancel reply")
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.replying_to = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The row of removable previews for the files staged to be sent, like
    /// Discord's attachment tray.
    fn render_attachment_previews(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .px_3()
            .pt_3()
            .children(self.pending_attachments.iter().map(|attachment| {
                let id = attachment.id;
                div()
                    .relative()
                    .flex_shrink_0()
                    .child(self.render_attachment_preview(attachment, cx))
                    .child(
                        div().absolute().top(px(4.)).right(px(4.)).child(
                            Button::new(("remove-attachment", id))
                                .icon(IconName::Close)
                                .danger()
                                .xsmall()
                                .tooltip("Remove attachment")
                                .on_click(cx.listener(move |this, _, _window, cx| {
                                    this.remove_attachment(id, cx);
                                })),
                        ),
                    )
            }))
    }

    /// The preview for one staged attachment: the image itself when it is one,
    /// otherwise a card with the file's name and size.
    fn render_attachment_preview(
        &self,
        attachment: &PendingAttachment,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let Some(image) = attachment.data.image() else {
            return v_flex()
                .h(px(120.))
                .w(px(160.))
                .p_3()
                .gap_2()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(theme.muted.opacity(0.5))
                .border_1()
                .border_color(theme.border)
                .child(
                    Icon::new(IconName::File)
                        .size_8()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_xs()
                        .text_center()
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format_size(attachment.data.bytes().len() as u64)),
                )
                .into_any_element();
        };

        img(image)
            .h(px(120.))
            .max_w(px(200.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .into_any_element()
    }
}
