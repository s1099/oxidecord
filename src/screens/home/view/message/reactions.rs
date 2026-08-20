//! The row of reaction pills under a message's content.

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex};

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Clicking a pill adds or removes the current user's reaction; the ones
    /// they already reacted with are highlighted, like Discord.
    pub(super) fn render_reactions(
        &self,
        message: &discord::Message,
        cx: &Context<Self>,
    ) -> impl IntoElement {
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
                        Some(url) => img(url)
                            .image_cache(&self.image_cache)
                            .size(px(16.))
                            .into_any_element(),
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
}
