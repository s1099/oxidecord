use gpui::{
    Context, IntoElement, ParentElement, Pixels, Render, Styled, Window, div,
};
use gpui_component::label::Label;
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem};
use gpui_component::Side;
use std::sync::{Arc, Mutex};
use crate::app::AppState;

pub struct ChannelView {
    app: Arc<Mutex<AppState>>,
}

impl ChannelView {
    pub fn new(_window: &mut Window, app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self { app }
    }

    fn render_server_list(&self) -> impl IntoElement {
        let guilds = self.app.lock().map(|app| app.guilds.clone()).unwrap_or_default();

        let mut menu = SidebarMenu::new();
        for guild in &guilds {
            let guild_name = guild.name.clone();
            
            menu = menu.child(
                SidebarMenuItem::new(guild_name.clone())
            );
        }

        Sidebar::new(Side::Left)
            .width(Pixels::from(72u32))
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

    fn render_channels_list(&self) -> impl IntoElement {
        Sidebar::new(Side::Left)
            .width(Pixels::from(240u32))
            .header(
                SidebarHeader::new()
                    .p_4()
                    .child(Label::new("Channels"))
            )
            .child(
                SidebarGroup::new("Text Channels")
                    .child(
                        SidebarMenu::new()
                            .child(SidebarMenuItem::new("general"))
                            .child(SidebarMenuItem::new("random"))
                    )
            )
    }

    fn render_message_view(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .bg(gpui::rgb(0x36393f))
            .child(
                div()
                    .p_4()
                    .border_b(gpui::px(1.0))
                    .border_color(gpui::rgb(0x2f3136))
                    .child(Label::new("Channel Name"))
            )
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .child(
                        div()
                            .text_color(gpui::rgb(0xdcddde))
                            .child("Messages will appear here...")
                    )
            )
    }
}

impl Render for ChannelView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .h_full()
            .child(self.render_server_list())
            .child(self.render_channels_list())
            .child(self.render_message_view())
    }
}

