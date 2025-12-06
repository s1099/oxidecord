use gpui::{
    Context, IntoElement, ParentElement, Pixels, Render, Styled, Window,
};
use gpui_component::label::Label;
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem};
use gpui_component::Side;
use std::sync::{Arc, Mutex};
use crate::app::AppState;

pub struct ServersView {
    app: Arc<Mutex<AppState>>,
}

impl ServersView {
    pub fn new(_window: &mut Window, app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self { app }
    }
}

impl Render for ServersView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let guilds = self.app.lock().map(|app| app.guilds.clone()).unwrap_or_default();

        let mut menu = SidebarMenu::new();
        for guild in &guilds {
            let guild_name = guild.name.clone();
            
            menu = menu.child(
                SidebarMenuItem::new(guild_name.clone())
            );
        }

        Sidebar::new(Side::Left)
            .width(Pixels::from(280u32))
            .header(
                SidebarHeader::new()
                    .p_4()
                    .child(Label::new("Oxidecord"))
            )
            .child(
                SidebarGroup::new("Servers")
                    .child(menu)
            )
    }
}
