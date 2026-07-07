use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar, divider::Divider, tooltip::Tooltip, v_flex, ActiveTheme as _,
};

use super::HomeScreen;

impl HomeScreen {
    pub(super) fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_guild;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let rail_border = theme.sidebar_border;
        let logo_bg = rgb(0x313338);
        let selected_bg = theme.sidebar_accent;

        v_flex()
            .id("server-rail")
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .py_3()
            .gap_3()
            .bg(rail_bg)
            .border_r_1()
            .border_color(rail_border)
            .child(
                div()
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(logo_bg)
                    .child(
                        img(Arc::new(Image::from_bytes(
                            ImageFormat::Svg,
                            crate::constants::DISCORD_ICON.as_bytes().to_vec(),
                        )))
                        .size(px(28.)),
                    ),
            )
            .child(Divider::horizontal().w(px(32.)))
            .child(
                v_flex()
                    .id("guild-list")
                    .flex_1()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .overflow_y_scroll()
                    .children(self.guilds.iter().map(|guild| {
                        let guild_id = guild.id;
                        let guild_name = guild.name.clone();
                        let is_selected = selected == Some(guild_id);

                        let mut avatar = Avatar::new().name(guild_name.clone());
                        if let Some(icon_url) = guild.icon_url.clone() {
                            avatar = avatar.src(icon_url);
                        }

                        div()
                            .id(("guild", guild_id.get()))
                            .cursor_pointer()
                            .p(px(4.))
                            .rounded(px(16.))
                            .when(is_selected, |this| this.bg(selected_bg))
                            .child(avatar)
                            .tooltip(move |window, cx| {
                                Tooltip::new(guild_name.clone()).build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_guild(guild_id, window, cx);
                            }))
                    })),
            )
    }
}
