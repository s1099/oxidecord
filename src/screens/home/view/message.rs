//! A single message row: its header, content, attachments, and hover toolbar.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconNamed as _, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, h_flex, v_flex,
};

use crate::discord;
use crate::screens::home::{HomeScreen, ReplyTarget};

use super::text::render_message_text;
use super::{GROUP_GAP, MESSAGE_PADDING_X};

/// Largest inline preview an image is scaled down to, in pixels. Discord uses
/// similar bounds; the aspect ratio is preserved within them.
const MAX_IMAGE_WIDTH: f32 = 400.;
const MAX_IMAGE_HEIGHT: f32 = 300.;

fn render_image_attachment(image: &discord::ImageAttachment) -> impl IntoElement {
    let mut element = img(image.url.clone())
        .rounded(px(8.))
        .max_w(px(MAX_IMAGE_WIDTH));
    match (image.width, image.height) {
        // With intrinsic dimensions we can lay out the exact scaled box, so
        // the message doesn't reflow once the image finishes loading.
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_IMAGE_WIDTH / width)
                .min(MAX_IMAGE_HEIGHT / height)
                .min(1.);
            element = element.w(px(width * scale)).h(px(height * scale));
        }
        _ => element = element.max_h(px(MAX_IMAGE_HEIGHT)),
    }
    element
}

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
                    v_flex()
                        .gap_1()
                        .children(message.images.iter().map(render_image_attachment)),
                )
            })
            .when(!message.reactions.is_empty(), |this| {
                this.child(self.render_reactions(message, cx))
            })
            .into_any_element();

        // The list can't pad its items (they overflow its padding), so each
        // message carries its own horizontal padding and a full width with
        // `min_w_0` so long lines wrap instead of running off the right edge.
        // The gap between two author groups is split evenly between them, so
        // each message's hover highlight extends symmetrically into it rather
        // than the whole gap highlighting with the message below.
        let inner: AnyElement = if !show_header {
            div()
                .w_full()
                .min_w_0()
                .pl(px(MESSAGE_PADDING_X + 52.))
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
        } else {
            let mut avatar = Avatar::new()
                .name(message.author_name.clone())
                .with_size(px(40.));
            if let Some(avatar_url) = message.author_avatar_url.clone() {
                avatar = avatar.src(avatar_url);
            }

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

            // A reply sits as a quoted line above the avatar+name row, indented
            // to line up with the message content column.
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
                            .pl(px(52.))
                            .child(self.render_reply_preview(&reference, cx)),
                    )
                })
                .child(header_row)
                .into_any_element()
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

    /// The row of reaction pills under a message's content. Clicking one adds
    /// or removes the current user's reaction; the ones they already reacted
    /// with are highlighted, like Discord.
    fn render_reactions(&self, message: &discord::Message, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let message_id = message.id;

        h_flex()
            .flex_wrap()
            .gap_1()
            .pt(px(2.))
            .children(message.reactions.iter().map(|reaction| {
                let emoji = reaction.emoji.clone();
                let key = match &emoji {
                    discord::ReactionEmoji::Unicode(name) => name.clone(),
                    discord::ReactionEmoji::Custom { id, .. } => id.to_string(),
                };

                h_flex()
                    .id(SharedString::from(format!(
                        "reaction-{}-{key}",
                        message_id.get()
                    )))
                    .gap_1()
                    .h(px(22.))
                    .px(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if reaction.me {
                        theme.primary
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(if reaction.me {
                        theme.primary.opacity(0.2)
                    } else {
                        theme.accent
                    })
                    .cursor_pointer()
                    .hover(|this| this.border_color(theme.primary.opacity(0.6)))
                    .child(match emoji.image_url() {
                        Some(url) => img(url).size(px(16.)).into_any_element(),
                        None => match &emoji {
                            discord::ReactionEmoji::Unicode(name) => div()
                                .text_size(px(15.))
                                .child(name.clone())
                                .into_any_element(),
                            _ => div().into_any_element(),
                        },
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(if reaction.me {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            })
                            .child(reaction.count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_reaction(message_id, emoji.clone(), cx);
                    }))
            }))
    }

    /// The quoted "↱ <author> <preview>" line shown above a message that
    /// replies to another, aligned with the message's content column.
    fn render_reply_preview(
        &self,
        reference: &discord::MessageReference,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        let mut avatar = Avatar::new()
            .name(reference.author_name.clone())
            .with_size(px(16.));
        if let Some(avatar_url) = reference.author_avatar_url.clone() {
            avatar = avatar.src(avatar_url);
        }

        h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                Icon::default()
                    .path(IconName::Undo2.path())
                    .size_3()
                    .flex_shrink_0(),
            )
            .child(avatar)
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(reference.author_name.clone()),
            )
            .child(if reference.content.is_empty() {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .italic()
                    .child("Click to see attachment")
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(reference.content.clone())
            })
    }

    /// The floating reply / more-actions toolbar shown at the top-right of a
    /// message while it's hovered.
    fn render_message_toolbar(
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
