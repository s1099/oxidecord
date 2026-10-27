//! A single message row: its header, content, attachments, and hover toolbar.

mod attachment;
mod edit;
mod embed;
mod forward;
mod menu;
mod pins;
mod reactions;
mod reactors;
mod reply;
mod system;
mod toolbar;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord;
use crate::screens::home::selection::TextPart;
use crate::screens::home::state::{EditingMessage, MediaKey};
use crate::screens::home::view::{avatar, bot_tag};
use crate::screens::home::{HomeScreen, View};

use super::markdown::MarkdownOptions;
use super::{GROUP_GAP, MESSAGE_PADDING_X};
use toolbar::Hovered;

const AVATAR_SIZE: f32 = 40.;

/// Kept small so the highlight hugs the text, like Discord's.
const MESSAGE_PADDING_Y: f32 = 2.;

/// Indent for continuations and reply quotes, aligning both with the content column.
const CONTENT_INDENT: f32 = 52.;

/// Tuned by eye: with baselines level the smaller time reads low beside lowercase text,
/// so it sits a touch above, as in Discord.
const GUTTER_TIME_NUDGE: f32 = -0.6;

/// The box inline media is fitted into, attachments and embeds alike.
const MEDIA_MAX_WIDTH: f32 = 400.;
const MEDIA_MAX_HEIGHT: f32 = 300.;

const MENTION_BAR_WIDTH: f32 = 2.;

/// Space between a reply quote and the header below it, which its spine spans.
const REPLY_GAP: f32 = 4.;

/// `None` when Discord reported no dimensions; the element is then capped rather than sized.
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

#[derive(Clone, Copy)]
pub(super) struct Media<'a> {
    images: &'a [discord::ImageAttachment],
    videos: &'a [discord::VideoAttachment],
    files: &'a [discord::FileAttachment],
    embeds: &'a [discord::Embed],
    mentions: &'a [discord::MentionedUser],
}

impl<'a> Media<'a> {
    pub(super) fn of(message: &'a discord::Message) -> Self {
        Self {
            images: &message.images,
            videos: &message.videos,
            files: &message.files,
            embeds: &message.embeds,
            mentions: &message.mentions,
        }
    }

    pub(super) fn of_forward(forward: &'a discord::ForwardedMessage) -> Self {
        Self {
            images: &forward.images,
            videos: &forward.videos,
            files: &forward.files,
            embeds: &forward.embeds,
            mentions: &forward.mentions,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.images.is_empty()
            && self.videos.is_empty()
            && self.files.is_empty()
            && self.embeds.is_empty()
    }
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

        // A system notice names the users it's about; it doesn't ping them.
        let mentioned = message.system.is_none() && self.mentions_me(message);
        let lit = self.toolbar_shown(message.id);
        let inner = if let Some(event) = &message.system {
            self.render_system_message(message, event, cx)
        } else {
            let content = self.render_message_body(message, editing, cx);
            if show_header {
                self.render_with_header(message, content, cx)
            } else {
                render_continuation(content, lit.then(|| message.clock_label()), cx)
            }
        };

        // Hovering highlights the whole row and reveals the toolbar; using the toolbar counts as
        // hovering though it hangs over the row above.
        let message_id = message.id;
        // The message being replied to wears the reply's colour over any mention tint.
        let replying = self
            .replying_to
            .as_ref()
            .is_some_and(|target| target.message_id == message_id);
        // A just-jumped-to message is lit the same way briefly.
        let flashed = self.flashed_message == Some(message_id);
        let tint = if replying || flashed {
            Some(theme.primary)
        } else {
            mentioned.then_some(theme.warning)
        };
        let row = div()
            .id(("message", message_id.get()))
            .relative()
            .w_full()
            .min_w_0()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_hovered(Hovered::Message, message_id, *hovered, cx);
            }))
            .map(|this| match tint {
                // Deepens under the pointer rather than vanishing, so mentions stay visible.
                Some(tint) => this
                    .bg(tint.opacity(if lit { 0.16 } else { 0.1 }))
                    .hover(|this| this.bg(tint.opacity(0.16)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(px(MENTION_BAR_WIDTH))
                            .bg(tint),
                    ),
                None => this
                    .when(lit, |this| this.bg(theme.accent.opacity(0.4)))
                    .hover(|this| this.bg(theme.accent.opacity(0.4))),
            })
            // A message open for editing stays lit, so it's clear which one the box belongs to.
            .when(editing.is_some(), |this| this.bg(theme.accent.opacity(0.4)))
            // Alt-click marks the conversation unread from here.
            .when(message.delivery == discord::Delivery::Sent, |this| {
                this.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if event.modifiers().alt {
                        this.mark_unread(message_id, cx);
                    }
                }))
            })
            .child(inner);
        // Nothing on the toolbar or menu applies until the server knows the message. The toolbar is
        // built only for its own message: hidden on every other row it would be dozens of elements
        // thrown away each frame.
        let row = if message.delivery == discord::Delivery::Sent {
            // A system notice can only be reacted to, which its menu covers.
            let row = row.when(lit && message.system.is_none(), |row| {
                row.child(self.render_message_toolbar(message, cx))
            });
            self.message_context_menu(row, message_id, cx)
                .into_any_element()
        } else {
            row.into_any_element()
        };

        // The gap between author groups is outside the row, so the highlight stays tight.
        div()
            .w_full()
            .min_w_0()
            .when(show_header, |this| this.pt(px(GROUP_GAP)))
            .child(row)
            .into_any_element()
    }

    /// A regular message's text, media, reactions and delivery state.
    fn render_message_body(
        &self,
        message: &discord::Message,
        editing: Option<&EditingMessage>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let media = Media::of(message);
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
            .when(editing.is_none() && message.shows_text(), |this| {
                this.child(
                    self.render_markdown(
                        &message.markdown,
                        MarkdownOptions::new(format!("message-{}", message.id), &message.mentions)
                            .edited(message.edited_label())
                            .jumbo()
                            .selectable(message.id.get(), TextPart::Content),
                        cx,
                    ),
                )
            })
            .when(
                editing.is_none()
                    && message.content.is_empty()
                    && uploads.is_empty()
                    && message.forward.is_none()
                    && media.is_empty(),
                |this| {
                    this.child(
                        div()
                            .italic()
                            .text_color(theme.muted_foreground)
                            .child("(no text content)"),
                    )
                },
            )
            .when(!media.is_empty(), |this| {
                this.child(self.render_media(message.id.get(), media, cx))
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
            .when(
                matches!(message.delivery, discord::Delivery::Sending { .. }),
                |this| this.opacity(0.5),
            )
            .into_any_element();
        content
    }

    /// A message's images, videos, files, and embeds, stacked under its text.
    ///
    /// `message_id` keys the media's element ids and playback. A forward's media borrows its
    /// forwarding message's id: Discord sends a comment on a forward as a separate message, so
    /// a forward never carries media of its own to collide with.
    fn render_media(&self, message_id: u64, media: Media, cx: &Context<Self>) -> impl IntoElement {
        let Media {
            images,
            videos,
            files,
            embeds,
            mentions,
        } = media;
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
            .when(!files.is_empty(), |this| {
                this.child(v_flex().gap_1().children(files.iter().enumerate().map(
                    |(index, file)| self.render_file(file, MediaKey { message_id, index }, cx),
                )))
            })
            .when(!embeds.is_empty(), |this| {
                this.child(self.render_embeds(message_id, embeds, mentions, cx))
            })
    }

    /// Whether a message pings the signed-in user: a mention by name (a pinging reply counts,
    /// as it lists the replied-to author), of one of their roles, or a sent `@everyone`/`@here`.
    fn mentions_me(&self, message: &discord::Message) -> bool {
        let Some(self_id) = self.self_user_id else {
            return false;
        };
        if message.mention_everyone || message.mentions.iter().any(|user| user.id == self_id) {
            return true;
        }
        // Role mentions only exist in guilds, and the open guild is the message's own.
        self.selected_guild
            .filter(|_| self.view == View::Guild)
            .is_some_and(|guild_id| self.mentions_self_role(message, guild_id))
    }

    fn render_with_header(
        &self,
        message: &discord::Message,
        content: AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let author_name = self
            .author_name(message.author_id, &message.author_name)
            .to_owned();

        let avatar = avatar(
            author_name.clone(),
            message.author_avatar_url.clone(),
            px(AVATAR_SIZE),
        );

        // Handled on mouse-down rather than click so the card's dismiss layer doesn't see the same
        // press and close it again.
        let author_id = message.author_id;
        let profile_name = author_name.clone();
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
                        profile_name.clone(),
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
                                    .when_some(self.role_color(author_id), |this, color| {
                                        this.text_color(rgb(color))
                                    })
                                    .child(author_name),
                            )
                            .when(message.author_bot, |this| this.child(bot_tag(cx)))
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

/// A message continuing the author group above it, with its send time in the gutter while
/// lit. The list can't pad its items, so each carries its own padding, plus a full width with
/// `min_w_0` so long lines wrap.
fn render_continuation(
    content: AnyElement,
    time: Option<String>,
    cx: &Context<HomeScreen>,
) -> AnyElement {
    div()
        .relative()
        .w_full()
        .min_w_0()
        .pl(px(MESSAGE_PADDING_X + CONTENT_INDENT))
        .pr(px(MESSAGE_PADDING_X))
        .py(px(MESSAGE_PADDING_Y))
        .text_sm()
        .when_some(time, |this, time| {
            // Aligned to the content's first line box (`text_sm` at gpui's default phi line
            // height),
            // not the whole message: text centres in its line box, so the smaller time's baseline
            // would
            // sit above the content's; the nudge down closes that gap.
            this.child(
                div()
                    .absolute()
                    .top(px(MESSAGE_PADDING_Y + GUTTER_TIME_NUDGE))
                    .left(px(MESSAGE_PADDING_X))
                    .w(px(CONTENT_INDENT))
                    .text_center()
                    .text_size(rems(0.6875))
                    .line_height(rems(0.875 * 1.618_034))
                    .text_color(cx.theme().muted_foreground)
                    .child(time),
            )
        })
        .child(content)
        .into_any_element()
}
