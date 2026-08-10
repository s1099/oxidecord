//! The guild sidebar: the channel list, its categories, and the account panel
//! pinned below it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, collapsible::Collapsible, h_flex, skeleton::Skeleton, v_flex,
};

use crate::discord::Channel;

use crate::screens::home::HomeScreen;
use crate::screens::home::channels::{ChannelGroup, channel_icon_path};

impl HomeScreen {
    fn render_channel_row(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let channel_id = channel.id;
        let is_selected = self.selected_channel == Some(channel_id);

        h_flex()
            .id(("channel", channel_id.get()))
            .px_2()
            .py(px(5.))
            .gap_2()
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
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_4(),
            )
            .child(div().flex_1().truncate().child(channel.name.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_channel(channel_id, window, cx);
            }))
    }

    fn render_channel_group(&self, group: &ChannelGroup, cx: &Context<Self>) -> AnyElement {
        let Some(category) = &group.category else {
            return v_flex()
                .gap(px(2.))
                .children(
                    group
                        .channels
                        .iter()
                        .map(|channel| self.render_channel_row(channel, cx)),
                )
                .into_any_element();
        };

        let theme = cx.theme();
        let category_id = category.id;
        let collapsed = self.collapsed_categories.contains(&category_id);

        let header = h_flex()
            .id(("category", category_id.get()))
            .mt_2()
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .hover(|this| this.text_color(theme.sidebar_foreground))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size_3(),
            )
            .child(category.name.to_uppercase())
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed_categories.insert(category_id) {
                    this.collapsed_categories.remove(&category_id);
                }
                cx.notify();
            }));

        let mut collapsible = Collapsible::new()
            .open(!collapsed)
            .gap(px(2.))
            .child(header)
            .content(
                v_flex().gap(px(2.)).children(
                    group
                        .channels
                        .iter()
                        .map(|channel| self.render_channel_row(channel, cx)),
                ),
            );

        // Like Discord, keep the selected channel visible when its category
        // is collapsed.
        if collapsed {
            if let Some(selected) = group
                .channels
                .iter()
                .find(|channel| Some(channel.id) == self.selected_channel)
            {
                collapsible = collapsible.child(self.render_channel_row(selected, cx));
            }
        }

        collapsible.into_any_element()
    }

    /// The account panel pinned below the channel list, like Discord's user
    /// area.
    pub(super) fn render_user_panel(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let (name, username, avatar_src) = match &self.current_user {
            Some(user) => (
                user.name.clone(),
                format!("@{}", user.username),
                user.avatar_url.clone(),
            ),
            None => (String::new(), String::new(), None),
        };

        let mut avatar = Avatar::new().name(name.clone()).with_size(px(32.));
        if let Some(src) = avatar_src {
            avatar = avatar.src(src);
        }

        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(px(52.))
            .px_2()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar_accent.opacity(0.3))
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(username),
                    ),
            )
            .child(
                Button::new("user-settings")
                    .icon(IconName::Settings)
                    .ghost()
                    .small()
                    .tooltip("User Settings"),
            )
    }

    pub(crate) fn render_channel_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sidebar_border = theme.sidebar_border;
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
            const WIDTHS: [f32; 16] = [
                120., 88., 104., 72., 132., 96., 116., 80., 124., 92., 108., 76., 128., 100., 112.,
                84.,
            ];
            list = list.children(WIDTHS.iter().map(|&width| {
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
                            .child(guild_name),
                    ),
            )
            .child(list)
            .child(self.render_user_panel(cx))
    }
}
