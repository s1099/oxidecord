use crate::app::AppState;
use crate::services::discord::DiscordService;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, px};
use gpui_component::label::Label;
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem};
use gpui_component::{IconName, Side};
use std::sync::{Arc, Mutex};

pub struct ServerListView {
    app: Arc<Mutex<AppState>>,
}

impl ServerListView {
    pub fn new(app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self { app }
    }

    fn get_guilds(&self) -> Vec<crate::app::GuildInfo> {
        self.app
            .lock()
            .map(|app| app.guilds.clone())
            .unwrap_or_default()
    }

    fn get_selected_guild(
        &self,
    ) -> Option<twilight_model::id::Id<twilight_model::id::marker::GuildMarker>> {
        self.app
            .lock()
            .map(|app| app.selected_guild.clone())
            .unwrap_or(None)
    }
}

impl Render for ServerListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let guilds = self.get_guilds();
        let selected = self.get_selected_guild();

        Sidebar::new(Side::Left)
            .width(px(80.))
            .border_width(px(1.))
            .collapsible(false)
            .header(SidebarHeader::new().child(Label::new("DM's")))
            .child(
                SidebarGroup::new("").child(
                    SidebarMenu::new().children(
                        guilds
                            .iter()
                            .map(|guild| {
                                let guild_id = guild.id;
                                let guild_name = guild.name.clone();
                                let is_selected =
                                    selected.as_ref().map(|id| *id == guild_id).unwrap_or(false);
                                let app_clone = self.app.clone();

                                SidebarMenuItem::new(guild_name.clone())
                                    .active(is_selected)
                                    .icon(IconName::File)
                                    .on_click(cx.listener(move |_view, _, _, cx| {
                                        DiscordService::fetch_channels(app_clone.clone(), guild_id);
                                        cx.notify();
                                    }))
                            })
                            .collect::<Vec<_>>(),
                    ),
                ),
            )
    }
}
