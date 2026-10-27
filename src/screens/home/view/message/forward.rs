//! The quoted block a forwarded message shows: the original's content and
//! media behind a bar, under a "Forwarded" label, crediting where it came from.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::selection::TextPart;
use crate::screens::home::view::avatar;
use crate::screens::home::view::markdown::MarkdownOptions;

use super::Media;

const BAR_WIDTH: f32 = 4.;
const SOURCE_ICON_SIZE: f32 = 16.;

impl HomeScreen {
    pub(super) fn render_forward(
        &self,
        message_id: u64,
        forward: &discord::ForwardedMessage,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let media = Media::of_forward(forward);

        let label = h_flex()
            .gap_1()
            .items_center()
            .italic()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .child(Icon::default().path("icons/redo-2.svg").small())
            .child("Forwarded");

        // A forward out of a DM, or a guild the user has since left, has no guild name to show,
        // so it's credited with just the time.
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
            .when(forward.shows_text(), |this| {
                this.child(
                    self.render_markdown(
                        &forward.markdown,
                        MarkdownOptions::new(format!("forward-{message_id}"), &forward.mentions)
                            .edited(forward.edited_label())
                            .jumbo()
                            .selectable(message_id, TextPart::Forward),
                        cx,
                    ),
                )
            })
            .when(!media.is_empty(), |this| {
                this.child(self.render_media(message_id, media, cx))
            })
            .child(source)
    }
}
