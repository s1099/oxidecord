//! The left-hand server rail: the DMs button and the guild icons.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, avatar::Avatar, divider::Divider, tooltip::Tooltip, v_flex,
};

use crate::assets::icons::DISCORD_ICON;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(super) fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_guild;
        let in_dms = self.view == View::DirectMessages;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let rail_border = theme.sidebar_border;
        let logo_bg = rgb(0x313338);
        let selected_bg = theme.sidebar_accent;

        // The whole rail — the DMs icon, its separator, and the guild list —
        // scrolls as one column, so the icon isn't pinned above the list.
        v_flex()
            .id("server-rail")
            .w(px(72.))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .py_3()
            .gap_2()
            .bg(rail_bg)
            .border_r_1()
            .border_color(rail_border)
            .overflow_y_scroll()
            .track_scroll(self.rail_scroll.handle())
            .on_scroll_wheel(
                cx.listener(|this, event, window, _| this.rail_scroll.absorb(event, window)),
            )
            .child(
                div()
                    .id("home-dms")
                    .p(px(4.))
                    .rounded(px(16.))
                    .cursor_pointer()
                    .when(in_dms, |this| this.bg(selected_bg))
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
                                    DISCORD_ICON.as_bytes().to_vec(),
                                )))
                                .size(px(28.)),
                            ),
                    )
                    .tooltip(|window, cx| Tooltip::new("Direct Messages").build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_direct_messages(window, cx);
                    })),
            )
            .child(Divider::horizontal().w(px(32.)))
            .children(self.guilds.iter().map(|guild| {
                let guild_id = guild.id;
                let guild_name = guild.name.clone();
                let is_selected = !in_dms && selected == Some(guild_id);

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
                    .tooltip(move |window, cx| Tooltip::new(guild_name.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_guild(guild_id, window, cx);
                    }))
            }))
    }
}
