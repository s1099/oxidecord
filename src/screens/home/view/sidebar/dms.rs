//! The direct-message sidebar: one row per conversation.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, skeleton::Skeleton, v_flex};

use crate::discord::DirectMessage;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::content::mention_badge;
use crate::screens::home::view::user_panel::status_indicator;
use crate::screens::home::view::{avatar, unread_pill};
use crate::ui::depth::{Finish, Level, Lit as _, radius};
use crate::ui::theme;

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
        let unread = self.unreads.is_unread(channel_id);
        // Every message in a conversation the user is part of pings them,
        // which is what the count is of.
        let mentions = self.mentions.count(channel_id);

        // Only a 1:1 conversation has one person whose status to show.
        let status = dm.recipient_id.map(|user_id| self.user_status(user_id));
        let group: SharedString = format!("dm-{channel_id}").into();
        let backdrop = if is_selected {
            theme.sidebar_accent
        } else {
            theme.sidebar
        };
        let avatar =
            div()
                .relative()
                .flex_shrink_0()
                .child(avatar(dm.name.clone(), dm.avatar_url.clone(), px(32.)))
                .when_some(status, |this, status| {
                    this.child(div().absolute().right(px(-3.)).bottom(px(-3.)).child(
                        status_indicator(
                            status,
                            px(10.),
                            backdrop,
                            Some((group.clone(), theme.sidebar_accent)),
                            theme,
                        ),
                    ))
                });

        h_flex()
            .id(("dm", channel_id.get()))
            .group(group)
            .relative()
            .px_2()
            .py(px(6.))
            .gap_3()
            .items_center()
            .rounded(radius::ITEM)
            .cursor_pointer()
            .text_sm()
            .text_color(if is_selected || unread {
                theme.sidebar_accent_foreground
            } else {
                theme::secondary_text(cx)
            })
            .hover(|this| {
                this.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(is_selected, |this| {
                this.bg(theme.sidebar_accent)
                    .font_weight(FontWeight::MEDIUM)
                    .lit(Level::Raised, Finish::Subtle, px(44.), radius::ITEM, cx)
            })
            .when(unread, |this| {
                this.font_weight(FontWeight::SEMIBOLD)
                    .child(unread_pill(px(8.), cx))
            })
            .child(avatar)
            .child(div().flex_1().truncate().child(dm.name.clone()))
            .when(mentions > 0, |this| this.child(mention_badge(mentions, cx)))
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
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.dm_scroll.absorb(event, window)),
            )
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

        shell("Direct Messages", list, self.render_sidebar_footer(cx), cx)
    }
}
