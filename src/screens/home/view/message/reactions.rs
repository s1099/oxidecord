//! The row of reaction pills under a message's content.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;

use super::reactors::ReactionTooltip;
use crate::discord;
use crate::screens::home::HomeScreen;
use crate::ui::depth::{Finish, Level, Lit as _, radius};

const CHIP_HEIGHT: f32 = 24.;

impl HomeScreen {
    /// Own reactions are highlighted, like Discord; hovering a pill names who reacted.
    pub(super) fn render_reactions(
        &self,
        message: &discord::Message,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let message_id = message.id;
        let screen = cx.entity();

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
                    .h(px(CHIP_HEIGHT))
                    .px(px(6.))
                    .border_1()
                    .border_color(if reaction.me {
                        theme.primary
                    } else {
                        theme.secondary
                    })
                    // Discord's chips: a solid tint for your own reaction, a lifted gray for
                    // others'.
                    .bg(if reaction.me {
                        theme.primary.opacity(0.3)
                    } else {
                        theme.secondary
                    })
                    .lit(
                        Level::Raised,
                        Finish::Subtle,
                        px(CHIP_HEIGHT),
                        radius::ITEM - px(1.),
                        cx,
                    )
                    .rounded(radius::ITEM)
                    .cursor_pointer()
                    .hover(|this| {
                        this.bg(if reaction.me {
                            theme.primary.opacity(0.4)
                        } else {
                            theme.secondary_hover
                        })
                    })
                    .active(|this| this.shadow(Vec::new()))
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
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if reaction.me {
                                theme.primary.blend(theme.foreground.opacity(0.35))
                            } else {
                                theme.foreground.opacity(0.85)
                            })
                            .child(reaction.count.to_string()),
                    )
                    // Fetched as the pointer arrives, so the names are usually
                    // in by the time the tooltip's delay is up.
                    .on_hover(cx.listener({
                        let emoji = emoji.clone();
                        move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.load_reactors(message_id, emoji.clone(), false, cx);
                            }
                        }
                    }))
                    .tooltip({
                        let emoji = emoji.clone();
                        let screen = screen.clone();
                        move |_, cx| {
                            let (screen, emoji) = (screen.clone(), emoji.clone());
                            cx.new(|cx| ReactionTooltip::new(screen, message_id, emoji, cx))
                                .into()
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_reaction(message_id, emoji.clone(), cx);
                    }))
            }))
    }
}
