//! The floating action toolbar revealed while a message is hovered.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, button::Button, button::ButtonVariants as _, h_flex,
};

use crate::discord;
use crate::screens::home::{HomeScreen, ReplyTarget};

impl HomeScreen {
    /// Sits at the top-right of the message, shown only while `group_name` —
    /// the message row's hover group — is hovered.
    pub(super) fn render_message_toolbar(
        &self,
        message: &discord::Message,
        group_name: &SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .absolute()
            .top(px(-16.))
            .right(px(12.))
            .invisible()
            .group_hover(group_name.clone(), |this| this.visible())
            .child(
                h_flex()
                    .gap(px(2.))
                    .p(px(2.))
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.))
                    .shadow_md()
                    .child(
                        Button::new(("message-reply", message.id.get()))
                            .icon(IconName::Undo2)
                            .ghost()
                            .small()
                            .tooltip("Reply")
                            .on_click(cx.listener({
                                let target = ReplyTarget {
                                    message_id: message.id,
                                    author_name: message.author_name.clone(),
                                };
                                move |this, _, window, cx| {
                                    this.replying_to = Some(target.clone());
                                    // Jump straight to composing, like Discord.
                                    this.message_input.focus_handle(cx).focus(window);
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new(("message-more", message.id.get()))
                            .icon(IconName::Ellipsis)
                            .ghost()
                            .small()
                            .tooltip("More"),
                    ),
            )
    }
}
