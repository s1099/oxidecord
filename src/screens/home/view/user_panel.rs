//! The account panel pinned below the sidebar, like Discord's user area.
//! Shared by the channel sidebar and the DM sidebar.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, avatar::Avatar, button::Button,
    button::ButtonVariants as _, h_flex, v_flex,
};

use crate::screens::home::HomeScreen;
use crate::ui::settings;

impl HomeScreen {
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
                    .tooltip("User Settings")
                    .on_click(|_, window, cx| settings::open(window, cx)),
            )
    }
}
