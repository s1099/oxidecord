use std::ops::Range;
use std::sync::LazyLock;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconNamed as _, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, divider::Divider, h_flex, input::Input, skeleton::Skeleton,
    spinner::Spinner, v_flex,
};
use regex::Regex;

use crate::discord::{self, Channel};

use super::channels::channel_icon_path;
use super::{HomeScreen, View};

/// Matches `http`/`https` URLs, running each up to the next whitespace or
/// angle bracket. Trailing prose punctuation is trimmed separately.
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>]+").expect("valid url regex"));

/// An `http`/`https` URL found in message text: the byte range it occupies in
/// the content and the link target itself.
struct Link {
    range: Range<usize>,
    url: String,
}

/// Finds every `http`/`https` URL in `text`, returning each as the byte range
/// it occupies plus the URL string, in order of appearance. Trailing
/// punctuation that usually belongs to the surrounding prose rather than the
/// link (a sentence's period, a wrapping paren, ...) is left out of the match.
fn find_links(text: &str) -> Vec<Link> {
    URL_REGEX
        .find_iter(text)
        .map(|m| {
            let url = m.as_str().trim_end_matches(|c| {
                matches!(
                    c,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
                )
            });
            Link {
                range: m.start()..m.start() + url.len(),
                url: url.to_string(),
            }
        })
        .collect()
}

/// Renders message text with any `http`/`https` URLs shown in the theme's link
/// colour, underlined, and clickable — a click opens the URL in the default
/// browser. Text without links renders as a plain string.
fn render_message_text(id: u64, content: &str, link_color: Hsla) -> AnyElement {
    let links = find_links(content);
    if links.is_empty() {
        return div()
            .w_full()
            .min_w_0()
            .child(content.to_string())
            .into_any_element();
    }

    let highlight = HighlightStyle {
        color: Some(link_color),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(link_color),
            wavy: false,
        }),
        ..Default::default()
    };
    let ranges: Vec<Range<usize>> = links.iter().map(|link| link.range.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|link| link.url).collect();
    let highlights: Vec<(Range<usize>, HighlightStyle)> = ranges
        .iter()
        .map(|range| (range.clone(), highlight))
        .collect();

    // `with_highlights` computes the plain runs from the ambient text style at
    // layout time, so the non-link text keeps the surrounding size and colour;
    // only the link ranges get the highlight overlaid.
    let styled = StyledText::new(content.to_string()).with_highlights(highlights);
    div()
        .w_full()
        .min_w_0()
        .child(
            InteractiveText::new(("message-content", id), styled).on_click(
                ranges,
                move |ix, _window, cx| {
                    if let Some(url) = urls.get(ix) {
                        cx.open_url(url);
                    }
                },
            ),
        )
        .into_any_element()
}

/// Horizontal padding, in pixels, on either side of the message list
const MESSAGE_PADDING_X: f32 = 16.;

/// Vertical gap, in pixels, between two consecutive author groups. Split evenly
/// between the bottom of the group above and the top of the group below so each
/// message's hover highlight extends symmetrically into the gap.
const GROUP_GAP: f32 = 16.;

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

    fn render_message(
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
            // Show placeholder text only when there's nothing else to render.
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
            .into_any_element();

        // Consecutive messages from the same author share one header, like
        // Discord; align follow-ups with the content column (avatar + gap).
        // The list itself can't pad its items (they overflow its padding), so
        // each message carries its own horizontal padding and a full width with
        // `min_w_0` so long lines wrap instead of running off the right edge.
        // The gap between two author groups is split evenly: the last message of
        // a group carries the bottom half and the first message of the next group
        // carries the top half. That way each message's hover highlight extends
        // symmetrically into the gap, instead of the whole gap sitting on top of
        // (and only highlighting with) the message that starts the new group.
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
            // to line up with the message content column, like Discord.
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
        // Consecutive messages from the same author share one header, except
        // replies always show theirs so the quoted preview has room to sit
        // above the message, like Discord.
        let show_header = ix == 0
            || message.reply.is_some()
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        // Whether the following message begins a new author group, which mirrors
        // the `show_header` logic applied to `ix + 1`: it starts a group if it's a
        // reply or has a different author than this message.
        let next_starts_group = self
            .messages
            .get(ix + 1)
            .is_some_and(|next| next.reply.is_some() || next.author_id != message.author_id);
        self.render_message(message, show_header, next_starts_group, cx)
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

    /// The preview for one staged attachment: the image itself when it is one,
    /// otherwise a card with the file's name and size.
    fn render_attachment_preview(
        &self,
        attachment: &super::PendingAttachment,
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
                        .child(super::format_size(attachment.data.bytes().len() as u64)),
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

    /// The row of removable previews for the files staged to be sent, shown
    /// inside the composer above the input, like Discord's attachment tray.
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
