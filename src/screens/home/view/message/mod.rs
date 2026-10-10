//! A single message row: its header, content, attachments, and hover toolbar.

mod attachment;
mod edit;
mod embed;
mod forward;
mod reactions;
mod reply;
mod toolbar;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::discord;
use crate::screens::home::state::MediaKey;
use crate::screens::home::view::avatar;
use crate::screens::home::{HomeScreen, View};

use super::markdown::MarkdownOptions;
use super::{GROUP_GAP, MESSAGE_PADDING_X};

/// The avatar beside a group's first message.
const AVATAR_SIZE: f32 = 40.;

/// Space above and below each message, inside its hover highlight. Kept small
/// so the highlight hugs the text, like Discord's.
const MESSAGE_PADDING_Y: f32 = 2.;

/// How far a continuation message and a reply quote are indented, so both line
/// up with the content column beside the avatar.
const CONTENT_INDENT: f32 = 52.;

/// The box inline media is fitted into, attachments and embeds alike. Discord
/// uses similar bounds; the aspect ratio is preserved within them.
const MEDIA_MAX_WIDTH: f32 = 400.;
const MEDIA_MAX_HEIGHT: f32 = 300.;

/// The bar down the left edge of a message that mentions the user.
const MENTION_BAR_WIDTH: f32 = 2.;

/// Space between a reply quote and the header below it, which its spine spans.
const REPLY_GAP: f32 = 4.;

/// Scales reported dimensions down into a box, keeping their shape. `None`
/// when Discord didn't report them, in which case the element is capped rather
/// than sized.
fn fit_within(
    width: Option<u32>,
    height: Option<u32>,
    max_width: f32,
    max_height: f32,
) -> Option<(f32, f32)> {
    let (width, height) = (width? as f32, height? as f32);
    if width <= 0. || height <= 0. {
        return None;
    }
    let scale = (max_width / width).min(max_height / height).min(1.);
    Some((width * scale, height * scale))
}

impl HomeScreen {
    pub(super) fn render_message(
        &self,
        message: &discord::Message,
        show_header: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.message_id == message.id);

        let mentioned = self.mentions_me(message);
        let has_images = !message.images.is_empty();
        let has_videos = !message.videos.is_empty();
        let has_embeds = !message.embeds.is_empty();
        let uploads = match &message.delivery {
            discord::Delivery::Sending { uploads } => uploads.as_slice(),
            _ => &[],
        };
        let failed = message.delivery == discord::Delivery::Failed;
        let content: AnyElement = v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .when_some(message.forward.as_ref(), |this, forward| {
                this.child(self.render_forward(message.id.get(), forward, cx))
            })
            .when_some(editing, |this, editing| {
                this.child(self.render_edit_box(editing, cx))
            })
            .when(editing.is_none() && !message.content.is_empty(), |this| {
                this.child(
                    self.render_markdown(
                        &message.markdown,
                        MarkdownOptions::new(format!("message-{}", message.id), &message.mentions)
                            .edited(message.edited_label())
                            .jumbo(),
                        cx,
                    ),
                )
            })
            .when(
                editing.is_none()
                    && message.content.is_empty()
                    && uploads.is_empty()
                    && message.forward.is_none()
                    && !has_images
                    && !has_videos
                    && !has_embeds,
                |this| {
                    this.child(
                        div()
                            .italic()
                            .text_color(theme.muted_foreground)
                            .child("(no text content)"),
                    )
                },
            )
            .when(has_images || has_videos || has_embeds, |this| {
                this.child(self.render_media(
                    message.id.get(),
                    &message.images,
                    &message.videos,
                    &message.embeds,
                    &message.mentions,
                    cx,
                ))
            })
            .children(uploads.iter().map(|filename| {
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("Uploading {filename}…"))
            }))
            .when(!message.reactions.is_empty(), |this| {
                this.child(self.render_reactions(message, cx))
            })
            .when(failed, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child("Message failed to send."),
                )
            })
            // Dimmed until the server has it, as Discord shows a message in
            // flight.
            .when(
                matches!(message.delivery, discord::Delivery::Sending { .. }),
                |this| this.opacity(0.5),
            )
            .into_any_element();

        let inner = if show_header {
            self.render_with_header(message, content, cx)
        } else {
            render_continuation(content)
        };

        // Hovering anywhere over the row highlights its whole width and reveals
        // the floating action toolbar, like Discord. Using the toolbar counts
        // as hovering, though it hangs over the row above.
        let message_id = message.id;
        let group_name = SharedString::from(format!("message-{}", message_id.get()));
        let mention_tint = theme.warning;
        let lit = self.toolbar_shown(message_id);
        let row = div()
            .id(("message", message_id.get()))
            .group(group_name.clone())
            .relative()
            .w_full()
            .min_w_0()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_message = Some(message_id);
                } else if this.hovered_message == Some(message_id) {
                    this.hovered_message = None;
                }
                cx.notify();
            }))
            .map(|this| {
                if mentioned {
                    // A mention keeps its tint under the pointer, only deeper,
                    // so hovering doesn't hide which messages were for you.
                    this.bg(mention_tint.opacity(if lit { 0.16 } else { 0.1 }))
                        .hover(|this| this.bg(mention_tint.opacity(0.16)))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(px(MENTION_BAR_WIDTH))
                                .bg(mention_tint),
                        )
                } else {
                    this.when(lit, |this| this.bg(theme.accent.opacity(0.4)))
                        .hover(|this| this.bg(theme.accent.opacity(0.4)))
                }
            })
            // A message open for editing stays lit while the pointer is
            // elsewhere, so it's clear which one the box belongs to.
            .when(editing.is_some(), |this| this.bg(theme.accent.opacity(0.4)))
            .child(inner)
            // Nothing on the toolbar applies until the server knows the message.
            .when(message.delivery == discord::Delivery::Sent, |this| {
                this.child(self.render_message_toolbar(message, &group_name, cx))
            });

        // The gap between author groups sits outside the row, so it never
        // lights up with the highlight and the highlight stays tight.
        div()
            .w_full()
            .min_w_0()
            .when(show_header, |this| this.pt(px(GROUP_GAP)))
            .child(row)
            .into_any_element()
    }

    /// A message's images, videos, and embeds, stacked under its text.
    ///
    /// `message_id` keys the media's element ids and playback. A forward's
    /// media borrows its forwarding message's id: Discord sends any comment
    /// on a forward as a message of its own, so a forward never carries media
    /// of its own for those keys to collide with.
    fn render_media(
        &self,
        message_id: u64,
        images: &[discord::ImageAttachment],
        videos: &[discord::VideoAttachment],
        embeds: &[discord::Embed],
        mentions: &[discord::MentionedUser],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .when(!images.is_empty(), |this| {
                this.child(v_flex().gap_1().children(images.iter().enumerate().map(
                    |(index, image)| self.render_image(image, MediaKey { message_id, index }, cx),
                )))
            })
            .when(!videos.is_empty(), |this| {
                this.child(v_flex().gap_1().children(videos.iter().enumerate().map(
                    |(index, video)| self.render_video(video, MediaKey { message_id, index }, cx),
                )))
            })
            .when(!embeds.is_empty(), |this| {
                this.child(self.render_embeds(message_id, embeds, mentions, cx))
            })
    }

    /// Whether a message pings the signed-in user, which Discord marks by
    /// tinting its row: a mention of them by name (a reply that pings counts,
    /// since it lists the replied-to author among the mentions), of one of
    /// their roles, or an `@everyone` or `@here` that went out.
    fn mentions_me(&self, message: &discord::Message) -> bool {
        let Some(self_id) = self.self_user_id else {
            return false;
        };
        if message.mention_everyone || message.mentions.iter().any(|user| user.id == self_id) {
            return true;
        }
        // Role mentions only exist in guilds, and the open guild is the
        // message's own.
        let roles = self
            .selected_guild
            .filter(|_| self.view == View::Guild)
            .and_then(|guild_id| self.self_roles.get(&guild_id));
        roles.is_some_and(|roles| {
            message
                .mention_roles
                .iter()
                .any(|role| roles.contains(role))
        })
    }

    /// The first message of an author group: the avatar, name, and timestamp
    /// over the content, with the reply quote above them when there is one.
    fn render_with_header(
        &self,
        message: &discord::Message,
        content: AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let avatar = avatar(
            message.author_name.clone(),
            message.author_avatar_url.clone(),
            px(AVATAR_SIZE),
        );

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
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.open_profile(
                        author_id,
                        author_name.clone(),
                        author_avatar_url.clone(),
                        event.position,
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
                                    .child(message.timestamp_label()),
                            ),
                    )
                    .child(div().w_full().min_w_0().text_sm().child(content)),
            );

        v_flex()
            .w_full()
            .min_w_0()
            .py(px(MESSAGE_PADDING_Y))
            .px(px(MESSAGE_PADDING_X))
            .gap(px(REPLY_GAP))
            .when_some(message.reply.clone(), |this, reference| {
                this.child(self.render_reply_preview(message.id.get(), &reference, cx))
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
fn render_continuation(content: AnyElement) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .pl(px(MESSAGE_PADDING_X + CONTENT_INDENT))
        .pr(px(MESSAGE_PADDING_X))
        .py(px(MESSAGE_PADDING_Y))
        .text_sm()
        .child(content)
        .into_any_element()
}
