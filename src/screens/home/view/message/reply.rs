//! The quoted line shown above a message that replies to another.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconNamed as _, Sizable as _, avatar::Avatar, h_flex,
};

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// The "↱ <author> <preview>" line, aligned with the message's content
    /// column.
    pub(super) fn render_reply_preview(
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
}
