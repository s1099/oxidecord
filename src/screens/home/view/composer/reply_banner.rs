//! The "Replying to <author>" strip atop the composer while a reply is pending.

use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::*;

use crate::screens::home::{HomeScreen, ReplyTarget};
use crate::ui::button::Button;

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
                    .id("reply-banner-target")
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|this| this.text_color(theme.foreground))
                    .on_click(cx.listener({
                        let message_id = target.message_id;
                        move |this, _, window, cx| this.jump_to_message(message_id, window, cx)
                    }))
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
