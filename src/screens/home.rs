use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar, divider::Divider, h_flex, tooltip::Tooltip, v_flex, ActiveTheme as _,
};
use twilight_model::id::{marker::GuildMarker, Id};

use crate::discord::{self, Guild};

pub struct HomeScreen {
    guilds: Vec<Guild>,
    selected_guild: Option<Id<GuildMarker>>,
    loading: bool,
    error: Option<String>,
}

impl HomeScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            guilds: Vec::new(),
            selected_guild: None,
            loading: true,
            error: None,
        };
        this.load_guilds(cx);
        this
    }

    fn load_guilds(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            self.loading = false;
            self.error = Some("No token found in auth.json. Please log in first.".into());
            return;
        };

        // `discord::fetch_guilds` runs its callback on a background Tokio
        // thread, but gpui's entity/async handles are `!Send`. Bridge the two
        // with a plain, `Send`-safe channel and let gpui's own (non-Send)
        // foreground task pick up the result.
        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_guilds(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(guilds) => {
                        this.selected_guild = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn render_server_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected_guild;
        let theme = cx.theme();
        let rail_bg = theme.sidebar;
        let rail_border = theme.sidebar_border;
        // Fixed dark badge: the white logo is baked into the SVG (img() can't
        // tint), and a dark background keeps it readable in either theme while
        // hiding the antialiased edge fringe from gpui's SVG rasterization.
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_guild = Some(guild_id);
                                cx.notify();
                            }))
                    })),
            )
    }

    fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Loading...")
                .into_any_element();
        }

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(theme.danger).child(error.clone()))
                .into_any_element();
        }

        let selected_name = self
            .selected_guild
            .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
            .map(|guild| guild.name.clone());

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(selected_name.unwrap_or_else(|| "Select a server".into())),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("soonTM."),
            )
            .into_any_element()
    }
}

impl Render for HomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_server_rail(cx))
            .child(self.render_content(cx))
    }
}
