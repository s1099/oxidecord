//! The guild sidebar: the channel list under the guild's name.

use gpui_kit::component::{ActiveTheme as _, h_flex, skeleton::Skeleton, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::HomeScreen;

use super::{TITLE_HEIGHT, shell, title_row};

/// Discord's banners are 16:9; the cap keeps a widened sidebar from giving it
/// half the column.
const BANNER_ASPECT: f32 = 9. / 16.;
const MAX_BANNER_HEIGHT: f32 = 150.;

const BADGE_SIZE: f32 = 16.;

/// Skeleton row widths, fixed so it doesn't reshuffle between frames.
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
        let appearance = self
            .selected_guild
            .and_then(|id| self.guild_data.appearances.get(&id))
            .cloned()
            .unwrap_or_default();

        let mut list = v_flex().px_2().py_2().gap(px(2.));

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

        let badge = appearance
            .verified
            .then(|| verified_badge(cx).into_any_element());
        let title = title_row(guild_name, badge);
        let scroller = v_flex()
            .id("channel-list")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(self.sidebar_scroll.handle())
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.sidebar_scroll.absorb(event, window)),
            );

        let Some(url) = appearance.banner_url else {
            return shell(
                Some(title.into_any_element()),
                scroller.child(list).into_any_element(),
                cx,
            );
        };
        // The banner scrolls with the list while the title stays pinned over it,
        // turning solid once the banner has slid out from under it.
        let banner_height = (self.sidebar_width * BANNER_ASPECT).min(px(MAX_BANNER_HEIGHT));
        let over_banner = banner_height + self.sidebar_scroll.offset() > px(TITLE_HEIGHT);
        let banner = div()
            .relative()
            .flex_shrink_0()
            .w_full()
            .h(banner_height)
            .child(
                img(url)
                    .image_cache(&crate::ui::avatar::cache(cx))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            // Keeps the name legible over a bright banner.
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(black().alpha(0.6), 0.),
                linear_color_stop(black().alpha(0.), 0.6),
            )));
        let title = title.absolute().top_0().left_0().right_0().map(|title| {
            if over_banner {
                title.text_color(white())
            } else {
                title.bg(theme.sidebar)
            }
        });
        let content = v_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(scroller.child(banner).child(list))
            .child(title);

        shell(None, content.into_any_element(), cx)
    }
}

fn verified_badge(cx: &App) -> impl IntoElement {
    div()
        .relative()
        .flex_shrink_0()
        .size(px(BADGE_SIZE))
        .child(
            svg()
                .path("icons/badge-fill.svg")
                .absolute()
                .size_full()
                .text_color(cx.theme().success),
        )
        .child(
            svg()
                .path("icons/badge-check-mark.svg")
                .absolute()
                .size_full()
                .text_color(white()),
        )
}
