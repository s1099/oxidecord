use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, divider::Divider, h_flex, input::Input, skeleton::Skeleton,
    spinner::Spinner, v_flex,
};

use crate::discord::{self, Channel};

use super::channels::channel_icon_path;
use super::{HomeScreen, View};

/// Horizontal padding, in pixels, on either side of the message list
const MESSAGE_PADDING_X: f32 = 16.;

/// A bottom-aligned column of placeholder message rows
fn messages_skeleton() -> impl IntoElement {
    const WIDTHS: [f32; 12] = [
        420., 280., 360., 200., 480., 320., 260., 440., 300., 380., 220., 460.,
    ];
    v_flex()
        .flex_1()
        .w_full()
        .justify_end()
        .gap_4()
        .py_2()
        .children(WIDTHS.iter().map(|&content_width| {
            h_flex()
                .w_full()
                .px(px(MESSAGE_PADDING_X))
                .gap_3()
                .items_start()
                .child(Skeleton::new().size(px(40.)).rounded_full())
                .child(
                    v_flex()
                        .flex_1()
                        .gap_2()
                        .child(Skeleton::new().w(px(120.)).h_4().rounded_md())
                        .child(Skeleton::new().w(px(content_width)).h_4().rounded_md()),
                )
        }))
}

/// Largest inline preview an image is scaled down to, in pixels. Discord uses
/// similar bounds; the aspect ratio is preserved within them.
const MAX_IMAGE_WIDTH: f32 = 400.;
const MAX_IMAGE_HEIGHT: f32 = 300.;

/// Renders one image attachment as a rounded, size-bounded preview.
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
        // Otherwise just cap the box and let the image size itself.
        _ => element = element.max_h(px(MAX_IMAGE_HEIGHT)),
    }
    element
}

impl HomeScreen {
    pub(super) fn render_channel_header(
        &self,
        channel: &Channel,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        h_flex()
            .h(px(48.))
            .w_full()
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(channel.name.clone()),
            )
            .when_some(
                // show only the first line and let `truncate` elide the rest.
                channel
                    .topic
                    .as_deref()
                    .and_then(|topic| topic.lines().find(|line| !line.trim().is_empty()))
                    .map(str::to_owned),
                |this, topic| {
                    this.child(Divider::vertical().h(px(24.))).child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(topic),
                    )
                },
            )
    }

    fn render_message(
        &self,
        message: &discord::Message,
        show_header: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let has_images = !message.images.is_empty();
        let content: AnyElement = v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            // Show placeholder text only when there's nothing else to render.
            .when(!message.content.is_empty(), |this| {
                this.child(div().w_full().min_w_0().child(message.content.clone()))
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
            .into_any_element();

        // Consecutive messages from the same author share one header, like
        // Discord; align follow-ups with the content column (avatar + gap).
        // The list itself can't pad its items (they overflow its padding), so
        // each message carries its own horizontal padding and a full width with
        // `min_w_0` so long lines wrap instead of running off the right edge.
        let inner: AnyElement = if !show_header {
            div()
                .w_full()
                .min_w_0()
                .pl(px(MESSAGE_PADDING_X + 52.))
                .pr(px(MESSAGE_PADDING_X))
                .py(px(1.))
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

            h_flex()
                .w_full()
                .pt(px(16.))
                .px(px(MESSAGE_PADDING_X))
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
                )
                .into_any_element()
        };

        // Wrap the row in a hover group: hovering anywhere over it highlights
        // the whole width and reveals the floating action toolbar, like Discord.
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

    /// The floating reply / more-actions toolbar shown at the top-right of a
    /// message while it's hovered. Hidden by default and revealed via the
    /// message's hover group.
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
                                let target = super::ReplyTarget {
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

    pub(super) fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return messages_skeleton().into_any_element();
        }

        if let Some(error) = &self.messages_error {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element();
        }

        let entity = cx.entity();
        let messages_list = list(self.messages_list.clone(), move |ix, _window, cx| {
            entity.update(cx, |this, cx| this.render_message_item(ix, cx))
        })
        .flex_1()
        .py_2();

        let mut container = v_flex().flex_1().min_h_0().w_full();
        if self.older_loading {
            container = container.child(
                h_flex()
                    .w_full()
                    .py_1()
                    .justify_center()
                    .child(Spinner::new().small().color(theme.muted_foreground)),
            );
        }

        // Route message images through our own cache instead of gpui's global
        // asset cache (which never evicts). Child `img` elements pick this up
        // via the cache stack. We clear it on channel switch, so image memory
        // is bounded to the messages currently on screen.
        image_cache(self.image_cache.clone())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(container.child(messages_list))
            .into_any_element()
    }

    fn render_message_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        // Consecutive messages from the same author share one header.
        let show_header = ix == 0
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        self.render_message(message, show_header, cx)
    }

    /// The "Replying to <author>" strip that sits atop the composer while a
    /// reply is pending, with a button to cancel the reply.
    fn render_reply_banner(&self, target: &super::ReplyTarget, cx: &Context<Self>) -> AnyElement {
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

    pub(super) fn render_message_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .w_full()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_4()
            .gap_1()
            .when_some(self.send_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                // Wrap the composer in our own rounded surface so the reply
                // banner and input read as one control. The input's own border
                // and bright focus ring are switched off in favour of this.
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
                    .child(
                        Input::new(&self.message_input)
                            .appearance(false)
                            .focus_bordered(false),
                    ),
            )
    }

    pub(super) fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.view == View::DirectMessages {
            return self.render_dm_content(cx);
        }

        if self.loading {
            return messages_skeleton().into_any_element();
        }

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(theme.danger).child(error.clone()))
                .into_any_element();
        }

        let Some(channel) = self.selected_channel_info().cloned() else {
            return messages_skeleton().into_any_element();
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_channel_header(&channel, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(cx))
            .into_any_element()
    }
}
