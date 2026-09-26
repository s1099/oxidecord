//! A single message row: its header, content, attachments, and hover toolbar.

mod attachment;
mod edit;
mod embed;
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

/// How far a continuation message and a reply quote are indented, so both line
/// up with the content column beside the avatar.
const CONTENT_INDENT: f32 = 52.;

/// The box inline media is fitted into, attachments and embeds alike. Discord
/// uses similar bounds; the aspect ratio is preserved within them.
const MEDIA_MAX_WIDTH: f32 = 400.;
const MEDIA_MAX_HEIGHT: f32 = 300.;

/// The bar down the left edge of a message that mentions the user.
const MENTION_BAR_WIDTH: f32 = 2.;

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
        next_starts_group: bool,
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
        let content: AnyElement = v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .when_some(editing, |this, editing| {
                this.child(self.render_edit_box(editing, cx))
            })
            .when(editing.is_none() && !message.content.is_empty(), |this| {
                this.child(
                    self.render_markdown(
                        &message.markdown,
                        MarkdownOptions::new(format!("message-{}", message.id), &message.mentions)
                            .edited(message.edited)
                            .jumbo(),
                        cx,
                    ),
                )
            })
            .when(
                editing.is_none()
                    && message.content.is_empty()
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
            .when(has_images, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .children(message.images.iter().enumerate().map(|(index, image)| {
                            self.render_image(
                                image,
                                MediaKey {
                                    message_id: message.id.get(),
                                    index,
                                },
                                cx,
                            )
                        })),
                )
            })
            .when(has_videos, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .children(message.videos.iter().enumerate().map(|(index, video)| {
                            self.render_video(
                                video,
                                MediaKey {
                                    message_id: message.id.get(),
                                    index,
                                },
                                cx,
                            )
                        })),
                )
            })
            .when(has_embeds, |this| {
                this.child(self.render_embeds(
                    message.id.get(),
                    &message.embeds,
                    &message.mentions,
                    cx,
                ))
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
        let mention_tint = theme.warning;
        div()
            .id(("message", message.id.get()))
            .group(group_name.clone())
            .relative()
            .w_full()
            .min_w_0()
            .map(|this| {
                if mentioned {
                    // A mention keeps its tint under the pointer, only deeper,
                    // so hovering doesn't hide which messages were for you.
                    this.bg(mention_tint.opacity(0.1))
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
                    this.hover(|this| this.bg(theme.accent.opacity(0.4)))
                }
            })
            // A message open for editing stays lit while the pointer is
            // elsewhere, so it's clear which one the box belongs to.
            .when(editing.is_some(), |this| this.bg(theme.accent.opacity(0.4)))
            .child(inner)
            .child(self.render_message_toolbar(message, &group_name, cx))
            .into_any_element()
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
        next_starts_group: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let avatar = avatar(
            message.author_name.clone(),
            message.author_avatar_url.clone(),
            px(40.),
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
                        .child(self.render_reply_preview(message.id.get(), &reference, cx)),
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
