//! The quoted block a forwarded message shows: the original's content and
//! media behind a bar, under a "Forwarded" label, crediting where it came from.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};

use crate::discord;
use crate::screens::home::view::avatar;
use crate::screens::home::view::markdown::MarkdownOptions;
use crate::screens::home::{HomeScreen, MessageTarget};

const BAR_WIDTH: f32 = 4.;
/// The source guild's icon in the footer.
const SOURCE_ICON_SIZE: f32 = 16.;

impl HomeScreen {
    pub(super) fn render_forward(
        &self,
        message_id: u64,
        forward: &discord::ForwardedMessage,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let has_media =
            !forward.images.is_empty() || !forward.videos.is_empty() || !forward.embeds.is_empty();

        // Clicking the header opens the original, when Discord said where it
        // went; forwards whose reference lacked the ids stay inert. A forward
        // out of a DM names no guild, which `jump_to_message` reads as a DM
        // channel.
        let origin = forward.message_id.zip(forward.channel_id);
        let forward_guild = forward.guild_id;
        let label_content = h_flex()
            .gap_1()
            .items_center()
            .italic()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(Icon::default().path("icons/redo-2.svg").small())
            .child("Forwarded");
        // `id` changes the element's type (`Div` -> `Stateful<Div>`), so the
        // clickable and inert shapes meet in `AnyElement`.
        let label = if let Some((message_id, channel_id)) = origin {
            label_content
                .id(("forward-source", message_id.get()))
                .cursor_pointer()
                .hover(|this| this.opacity(0.8))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.jump_to_message(
                        MessageTarget {
                            guild_id: forward_guild,
                            channel_id,
                            message_id,
                        },
                        window,
                        cx,
                    );
                }))
                .into_any_element()
        } else {
            label_content.into_any_element()
        };

        // Discord names the guild a forward came from. A forward out of a DM,
        // or out of a guild the user has since left, has no name to show, so
        // it's credited with just the time.
        let guild = forward
            .guild_id
            .and_then(|guild_id| self.guilds.iter().find(|guild| guild.id == guild_id));
        let source = h_flex()
            .gap_1()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .when_some(guild, |this, guild| {
                this.child(avatar(
                    guild.name.clone(),
                    guild.icon_url.clone(),
                    px(SOURCE_ICON_SIZE),
                ))
                .child(div().truncate().child(guild.name.clone()))
                .child("•")
            })
            .child(div().flex_shrink_0().child(forward.timestamp_label()));

        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .pl_3()
            .border_l(px(BAR_WIDTH))
            .border_color(theme.border)
            .child(label)
            .when(!forward.content.is_empty(), |this| {
                this.child(
                    self.render_markdown(
                        &forward.markdown,
                        MarkdownOptions::new(format!("forward-{message_id}"), &forward.mentions)
                            .edited(forward.edited_label())
                            .jumbo(),
                        cx,
                    ),
                )
            })
            .when(has_media, |this| {
                this.child(self.render_media(
                    message_id,
                    &forward.images,
                    &forward.videos,
                    &forward.embeds,
                    &forward.mentions,
                    cx,
                ))
            })
            .child(source)
    }
}
