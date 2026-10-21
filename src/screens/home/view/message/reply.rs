//! The quoted line shown above a message that replies to another.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex};

use crate::discord;
use crate::screens::home::view::avatar;
use crate::screens::home::view::markdown::MarkdownOptions;
use crate::screens::home::{HomeScreen, MessageTarget, View};

use super::{AVATAR_SIZE, CONTENT_INDENT, REPLY_GAP};

/// Height of the quoted line, which the spine's corner is centred on.
const LINE_HEIGHT: f32 = 18.;
const SPINE_WIDTH: f32 = 2.;
/// Clearance between the spine's end and the quoted author's avatar.
const SPINE_CLEARANCE: f32 = 4.;

impl HomeScreen {
    /// The "<author> <preview>" line, aligned with the message's content
    /// column, with a curved spine linking it down to the avatar like Discord.
    pub(super) fn render_reply_preview(
        &self,
        message_id: u64,
        reference: &discord::MessageReference,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let author_name = self
            .author_name(reference.author_id, &reference.author_name)
            .to_owned();

        let avatar = avatar(
            author_name.clone(),
            reference.author_avatar_url.clone(),
            px(16.),
        );

        // Starts at the avatar's centre and runs up and over to the quote. It
        // overhangs the row's bottom by the gap so it meets the avatar's top.
        let spine_left = AVATAR_SIZE / 2. - SPINE_WIDTH / 2.;
        let spine = div()
            .absolute()
            .left(px(spine_left))
            .w(px(CONTENT_INDENT - spine_left - SPINE_CLEARANCE))
            .top(px(LINE_HEIGHT / 2. - SPINE_WIDTH / 2.))
            .bottom(px(-REPLY_GAP))
            .border_t(px(SPINE_WIDTH))
            .border_l(px(SPINE_WIDTH))
            .border_color(theme.border)
            .rounded_tl(px(6.));

        let quote_jump = reference.message_id;
        h_flex()
            .relative()
            .w_full()
            .min_w_0()
            .h(px(LINE_HEIGHT))
            .pl(px(CONTENT_INDENT))
            .gap_1()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(spine)
            .child(avatar)
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when_some(self.role_color(reference.author_id), |this, color| {
                        this.text_color(rgb(color))
                    })
                    .child(author_name),
            )
            .child(if reference.content.is_empty() {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .italic()
                    .child("Click to see attachment")
            } else {
                // Formatted like the message itself, but squeezed onto one
                // line and with its own links inert: the whole line is one
                // click target, jumping to the original.
                div().flex_1().min_w_0().truncate().child(
                    self.render_inline_markdown(
                        &reference.preview,
                        MarkdownOptions::new(format!("reply-{message_id}"), &reference.mentions)
                            .interactive(false),
                        cx,
                    ),
                )
            })
            // Clicking the quote anywhere opens the original. The channel is
            // read at click time — it's the conversation this row was
            // rendered into — and the guild follows the open view.
            .id(("reply-quote", message_id))
            .cursor_pointer()
            .hover(|this| this.opacity(0.8))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                let Some(channel_id) = this.selected_channel else {
                    return;
                };
                let guild_id = (this.view == View::Guild)
                    .then_some(this.selected_guild)
                    .flatten();
                this.jump_to_message(
                    MessageTarget {
                        guild_id,
                        channel_id,
                        message_id: quote_jump,
                    },
                    window,
                    cx,
                );
            }))
    }
}
