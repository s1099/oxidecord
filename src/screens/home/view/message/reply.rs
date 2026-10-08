//! The quoted line shown above a message that replies to another.

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::screens::home::view::markdown::MarkdownOptions;

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

        let avatar = avatar(
            reference.author_name.clone(),
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
                // Formatted like the message itself, but squeezed onto one
                // line and not clickable: the whole line will be what jumps
                // to the original.
                div().flex_1().min_w_0().truncate().child(
                    self.render_inline_markdown(
                        &reference.preview,
                        MarkdownOptions::new(format!("reply-{message_id}"), &reference.mentions)
                            .interactive(false),
                        cx,
                    ),
                )
            })
    }
}
