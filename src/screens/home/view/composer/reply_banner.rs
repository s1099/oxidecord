//! The "Replying to <author>" strip atop the composer while a reply is pending.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, button::Button, button::ButtonVariants as _, h_flex,
};

use crate::screens::home::{HomeScreen, ReplyTarget};

impl HomeScreen {
    pub(super) fn render_reply_banner(
        &self,
        target: &ReplyTarget,
        cx: &Context<Self>,
    ) -> AnyElement {
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
}
