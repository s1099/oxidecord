use gpui::{AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::sidebar::{Sidebar, SidebarHeader, SidebarGroup, SidebarMenu, SidebarMenuItem, SidebarFooter};
use gpui_component::{IconName, Side};


pub struct ServersView {
    
}

impl ServersView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        println!("ServersView created");
        // let token = cx.new(|cx| {
        //     InputState::new(window, cx)
        //         .placeholder("Token")
        //         .masked(true)
        // });
        Self {  }
    }


}

impl Render for ServersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Sidebar::new(Side::Left)
        .header(
            SidebarHeader::new()
                .child("My Application")
        )
        .child(
            SidebarGroup::new("Navigation")
                .child(
                    SidebarMenu::new()
                        .child(
                            SidebarMenuItem::new("Dashboard")
                                .icon(IconName::LayoutDashboard)
                                .on_click(|_, _, _| println!("Dashboard clicked"))
                        )
                        .child(
                            SidebarMenuItem::new("Settings")
                                .icon(IconName::Settings)
                                .on_click(|_, _, _| println!("Settings clicked"))
                        )
                )
        )
        .footer(
            SidebarFooter::new()
                .child("User Profile")
        )    
    }
}
