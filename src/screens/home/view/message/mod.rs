//! A single message row: its header, content, attachments, and hover toolbar.

mod attachment;
mod reactions;
mod reply;
mod toolbar;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex, v_flex};

use crate::discord;
use crate::screens::home::HomeScreen;

use super::text::render_message_text;
use super::{GROUP_GAP, MESSAGE_PADDING_X};

/// How far a continuation message and a reply quote are indented, so both line
/// up with the content column beside the avatar.
const CONTENT_INDENT: f32 = 52.;

impl HomeScreen {
    pub(super) fn render_message(
        &self,
        message: &discord::Message,
        show_header: bool,
        next_starts_group: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let has_images = !message.images.is_empty();
        let content: AnyElement = v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .when(!message.content.is_empty(), |this| {
                this.child(render_message_text(
                    message.id.get(),
                    &message.content,
                    theme.link,
                ))
            })
            .when(message.content.is_empty() && !has_images, |this| {
                this.child(
                    div()
                        .italic()
                        .text_color(theme.muted_foreground)
                        .child("(no text content)"),
                )
            })
            .when(has_images, |this| {
                this.child(
                    v_flex().gap_1().children(
                        message
                            .images
                            .iter()
                            .map(|image| attachment::render_image(image, &self.image_cache)),
                    ),
                )
            })
            .when(!message.reactions.is_empty(), |this| {
                this.child(self.render_reactions(message, cx))
            })
            .into_any_element();

        let inner = if show_header {
            self.render_with_header(message, content, next_starts_group, cx)
        } else {
            render_continuation(content, next_starts_group)
        };

        // Hovering anywhere over the row highlights its whole width and reveals
        // the floating action toolbar, like Discord.
        let group_name = SharedString::from(format!("message-{}", message.id.get()));
        div()
            .id(("message", message.id.get()))
            .group(group_name.clone())
            .relative()
            .w_full()
            .min_w_0()
            .hover(|this| this.bg(theme.accent.opacity(0.4)))
            .child(inner)
            .child(self.render_message_toolbar(message, &group_name, cx))
            .into_any_element()
    }

    /// The first message of an author group: the avatar, name, and timestamp
    /// over the content, with the reply quote above them when there is one.
    fn render_with_header(
        &self,
        message: &discord::Message,
        content: AnyElement,
        next_starts_group: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let mut avatar = Avatar::new()
            .name(message.author_name.clone())
            .with_size(px(40.));
        if let Some(avatar_url) = message.author_avatar_url.clone() {
            avatar = avatar.src(avatar_url);
        }

        // Clicking the avatar opens the author's profile card, anchored at the
        // click. Handled on mouse-down rather than click so the card's dismiss
        // layer doesn't see the same press and close it again.
        let author_id = message.author_id;
        let author_name = message.author_name.clone();
        let author_avatar_url = message.author_avatar_url.clone();
        let avatar = div()
            .id(("message-avatar", message.id.get()))
            .flex_shrink_0()
            .cursor_pointer()
            .hover(|this| this.opacity(0.8))
            .child(avatar)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_profile(
                        author_id,
                        author_name.clone(),
                        author_avatar_url.clone(),
                        event.position,
                        window,
                        cx,
                    );
                }),
            );

        let header_row = h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .items_start()
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(message.author_name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(message.timestamp.clone()),
                            ),
                    )
                    .child(div().w_full().min_w_0().text_sm().child(content)),
            );

        v_flex()
            .w_full()
            .min_w_0()
            .pt(px(GROUP_GAP / 2.))
            .pb(px(if next_starts_group {
                GROUP_GAP / 2.
            } else {
                0.
            }))
            .px(px(MESSAGE_PADDING_X))
            .gap(px(2.))
            .when_some(message.reply.clone(), |this, reference| {
                this.child(
                    div()
                        .pl(px(CONTENT_INDENT))
                        .child(self.render_reply_preview(&reference, cx)),
                )
            })
            .child(header_row)
            .into_any_element()
    }
}

/// A message continuing the author group above it: just the content, indented
/// to sit under the first message's.
///
/// The list can't pad its items, so each message carries its own padding, plus
/// a full width with `min_w_0` so long lines wrap rather than overflow.
fn render_continuation(content: AnyElement, next_starts_group: bool) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .pl(px(MESSAGE_PADDING_X + CONTENT_INDENT))
        .pr(px(MESSAGE_PADDING_X))
        .pt(px(1.))
        .pb(px(if next_starts_group {
            GROUP_GAP / 2.
        } else {
            1.
        }))
        .text_sm()
        .child(content)
        .into_any_element()
}
