//! The "Replying to <author>" strip atop the composer while a reply is pending.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
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
        // Discord doesn't offer to ping yourself.
        let own = self
            .current_user
            .as_ref()
            .is_some_and(|user| user.id == target.author_id);
        let mention_color = if target.mention {
            theme.primary
        } else {
            theme.muted_foreground
        };

        h_flex()
            .w_full()
            .h(px(32.))
            // Matches the input row's right inset, so the X lines up with the emoji button.
            .pl_3()
            .pr_1()
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
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(!own, |this| {
                        this.child(
                            h_flex()
                                .id("reply-mention")
                                .px_1()
                                .items_center()
                                .gap_0p5()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(mention_color)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.8))
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    if let Some(target) = &mut this.replying_to {
                                        target.mention = !target.mention;
                                        cx.notify();
                                    }
                                }))
                                .child(
                                    Icon::default()
                                        .path("icons/at-sign.svg")
                                        .size_3p5()
                                        .text_color(mention_color),
                                )
                                .child(if target.mention { "ON" } else { "OFF" }),
                        )
                        .child(div().w_px().h(px(16.)).bg(theme.border))
                    })
                    .child(
                        Button::new("cancel-reply")
                            .icon(IconName::CircleX)
                            .ghost()
                            .small()
                            .tooltip("Cancel reply")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.replying_to = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}
