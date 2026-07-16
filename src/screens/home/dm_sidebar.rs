use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar, h_flex, skeleton::Skeleton, v_flex, ActiveTheme as _, Sizable as _,
};

use crate::discord::DirectMessage;

use super::HomeScreen;

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

    pub(super) fn render_dm_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sidebar_border = theme.sidebar_border;
        let danger = theme.danger;

        let mut list = v_flex()
            .id("dm-list")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .px_2()
            .py_2()
            .gap(px(2.));

        if self.dms_loading {
            const WIDTHS: [f32; 12] = [
                120., 88., 104., 72., 132., 96., 116., 80., 124., 92., 108., 76.,
            ];
            list = list.children(WIDTHS.iter().map(|&width| {
                h_flex()
                    .px_2()
                    .py(px(6.))
                    .gap_3()
                    .items_center()
                    .child(Skeleton::new().size(px(32.)).rounded_full())
                    .child(Skeleton::new().w(px(width)).h_4().rounded_md())
            }));
        } else if let Some(error) = &self.dms_error {
            list = list.child(div().px_2().text_sm().text_color(danger).child(error.clone()));
        } else if self.dms.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No conversations yet."),
            );
        } else {
            list = list.children(self.dms.iter().map(|dm| self.render_dm_row(dm, cx)));
        }

        v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(sidebar_border)
            .child(
                h_flex()
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_4()
                    .items_center()
                    .border_b_1()
                    .border_color(sidebar_border)
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Direct Messages"),
                    ),
            )
            .child(list)
    }

    fn render_dm_header(&self, dm: &DirectMessage, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let mut avatar = Avatar::new().name(dm.name.clone()).with_size(px(28.));
        if let Some(url) = dm.avatar_url.clone() {
            avatar = avatar.src(url);
        }

        h_flex()
            .h(px(48.))
            .w_full()
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(avatar)
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(dm.name.clone()),
            )
    }

    pub(super) fn render_dm_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        let Some(dm) = self.selected_dm_info().cloned() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select a conversation to start chatting.")
                .into_any_element();
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_dm_header(&dm, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(cx))
            .into_any_element()
    }
}
