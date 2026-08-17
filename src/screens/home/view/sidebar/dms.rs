//! The direct-message sidebar: one row per conversation.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, avatar::Avatar, h_flex, skeleton::Skeleton, v_flex,
};

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;

use super::shell;

/// Row widths for the loading skeleton, fixed so it doesn't reshuffle between
/// frames.
const SKELETON_WIDTHS: [f32; 12] = [
    120., 88., 104., 72., 132., 96., 116., 80., 124., 92., 108., 76.,
];

impl HomeScreen {
    fn render_dm_row(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = dm.id;
        let is_selected = self.selected_channel == Some(channel_id);

        let mut avatar = Avatar::new().name(dm.name.clone()).with_size(px(32.));
        if let Some(url) = dm.avatar_url.clone() {
            avatar = avatar.src(url);
        }

        h_flex()
            .id(("dm", channel_id.get()))
            .px_2()
            .py(px(6.))
            .gap_3()
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_sm()
            .text_color(if is_selected {
                theme.sidebar_accent_foreground
            } else {
                theme.muted_foreground
            })
            .when(is_selected, |this| this.bg(theme.sidebar_accent))
            .hover(|this| this.bg(theme.sidebar_accent.opacity(0.5)))
            .child(avatar)
            .child(div().flex_1().truncate().child(dm.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_channel(channel_id, window, cx);
            }))
    }

    pub(in crate::screens::home) fn render_dm_sidebar(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let danger = theme.danger;
        let muted_foreground = theme.muted_foreground;

        let mut list = v_flex()
            .id("dm-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(self.dm_scroll.handle())
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.dm_scroll.absorb()))
            .px_2()
            .py_2()
            .gap(px(2.));

        if self.dms_loading {
            list = list.children(SKELETON_WIDTHS.iter().map(|&width| {
                h_flex()
                    .px_2()
                    .py(px(6.))
                    .gap_3()
                    .items_center()
                    .child(Skeleton::new().size(px(32.)).rounded_full())
                    .child(Skeleton::new().w(px(width)).h_4().rounded_md())
            }));
        } else if let Some(error) = &self.dms_error {
            list = list.child(
                div()
                    .px_2()
                    .text_sm()
                    .text_color(danger)
                    .child(error.clone()),
            );
        } else if self.dms.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(muted_foreground)
                    .child("No conversations yet."),
            );
        } else {
            list = list.children(self.dms.iter().map(|dm| self.render_dm_row(dm, cx)));
        }

        shell("Direct Messages", list, self.render_user_panel(cx), cx)
    }
}
