//! The guild sidebar: the channel list under the guild's name.

use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, skeleton::Skeleton, v_flex};

use crate::screens::home::HomeScreen;

use super::shell;

/// Row widths for the loading skeleton, fixed so it doesn't reshuffle between
/// frames.
const SKELETON_WIDTHS: [f32; 16] = [
    120., 88., 104., 72., 132., 96., 116., 80., 124., 92., 108., 76., 128., 100., 112., 84.,
];

impl HomeScreen {
    pub(in crate::screens::home) fn render_channel_sidebar(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;

        let guild_name = self
            .selected_guild
            .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
            .map(|guild| guild.name.clone())
            .unwrap_or_default();

        let mut list = v_flex()
            .id("channel-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(self.sidebar_scroll.handle())
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.sidebar_scroll.absorb()))
            .px_2()
            .py_2()
            .gap(px(2.));

        if self.channels_loading || self.loading {
            list = list.children(SKELETON_WIDTHS.iter().map(|&width| {
                h_flex()
                    .px_2()
                    .py(px(5.))
                    .gap_2()
                    .items_center()
                    .child(Skeleton::new().size_4().rounded_md())
                    .child(Skeleton::new().w(px(width)).h_4().rounded_md())
            }));
        } else if let Some(error) = &self.channels_error {
            list = list.child(
                div()
                    .px_2()
                    .text_sm()
                    .text_color(danger)
                    .child(error.clone()),
            );
        } else {
            list = list.children(
                self.channel_groups
                    .iter()
                    .map(|group| self.render_channel_group(group, cx)),
            );
        }

        shell(guild_name, list, self.render_user_panel(cx), cx)
    }
}
