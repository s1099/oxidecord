use gpui::{
    Context, IntoElement, ParentElement, Pixels, Render, Styled, Window, div, prelude::*,
};
use gpui_component::label::Label;
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem};
use gpui_component::Side;
use gpui_component::{
    v_virtual_list, VirtualListScrollHandle,
    scroll::{Scrollbar, ScrollbarState, ScrollbarAxis},
};
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use gpui::{px, size};
use crate::app::AppState;
use crate::views::channels::ChannelsView;

pub struct ChannelView {
    app: Arc<Mutex<AppState>>,
    channels_view: Option<gpui::Entity<ChannelsView>>,
    message_scroll_handle: VirtualListScrollHandle,
    message_scroll_state: ScrollbarState,
}

impl ChannelView {
    pub fn new(_window: &mut Window, app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
            channels_view: None,
            message_scroll_handle: VirtualListScrollHandle::new(),
            message_scroll_state: ScrollbarState::default(),
        }
    }

    fn render_server_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let guilds = {
            let app = self.app.lock().unwrap();
            app.guilds.clone()
        };

        let mut menu = SidebarMenu::new();
        for guild in &guilds {
            let guild_id = guild.id;
            let guild_name = guild.name.clone();
            let app_clone = self.app.clone();
            
            let item = SidebarMenuItem::new(guild_name.clone())
                .on_click(cx.listener(move |_view, _button, _count, cx| {
                    crate::app::AppState::fetch_channels(app_clone.clone(), guild_id);
                    cx.notify();
                }));
            
            menu = menu.child(item);
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

    fn render_message_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (messages, channel_name) = {
            let app = self.app.lock().unwrap();
            let messages = app.messages.clone();
            let channel_name = app.channels.iter()
                .find(|ch| app.selected_channel.map(|id| ch.id == id).unwrap_or(false))
                .map(|ch| ch.name.clone())
                .unwrap_or_else(|| "Select a channel".to_string());
            (messages, channel_name)
        };

        let item_sizes = Rc::new(
            messages.iter()
                .map(|_| size(px(800.), px(60.))) // Fixed height for each message
                .collect::<Vec<_>>()
        );

        let messages_clone = messages.clone();

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
                    .child(Label::new(&channel_name))
            )
            .child(
                div()
                    .flex_1()
                    .relative()
                    .child(
                        if messages.is_empty() {
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .justify_center()
                                .p_4()
                                .child(
                                    div()
                                        .text_color(gpui::rgb(0xdcddde))
                                        .child("No messages yet. Select a channel to see messages.")
                                )
                                .into_any()
                        } else {
                            v_virtual_list(
                                cx.entity().clone(),
                                "messages-list",
                                item_sizes.clone(),
                                move |_view, visible_range, _, _cx| {
                                    visible_range
                                        .map(|ix| {
                                            if let Some(msg) = messages_clone.get(ix) {
                                                div()
                                                    .w_full()
                                                    .h(px(60.))
                                                    .p_4()
                                                    .border_b(gpui::px(1.0))
                                                    .border_color(gpui::rgb(0x2f3136))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap_1()
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .gap_2()
                                                                    .items_center()
                                                                    .child(
                                                                        div()
                                                                            .text_color(gpui::rgb(0xffffff))
                                                                            .font_weight(gpui::FontWeight::BOLD)
                                                                            .child(msg.author_name.clone())
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_color(gpui::rgb(0x72767d))
                                                                            .text_size(px(12.))
                                                                            .child(msg.timestamp.clone())
                                                                    )
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_color(gpui::rgb(0xdcddde))
                                                                    .child(msg.content.clone())
                                                            )
                                                    )
                                            } else {
                                                div().w_full().h(px(60.))
                                            }
                                        })
                                        .collect()
                                },
                            )
                            .track_scroll(&self.message_scroll_handle)
                            .into_any()
                        }
                    )
                    .child(
                        if !messages.is_empty() {
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .bottom_0()
                                .child(
                                    Scrollbar::both(&self.message_scroll_state, &self.message_scroll_handle)
                                        .axis(ScrollbarAxis::Vertical)
                                )
                                .into_any()
                        } else {
                            div().into_any()
                        }
                    )
            )
    }
}

impl Render for ChannelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        if self.channels_view.is_none() {
            self.channels_view = Some(cx.new(|cx| ChannelsView::new(app.clone(), cx)));
        }
        let channels_view_entity = self.channels_view.as_ref().unwrap();
        
        let server_list = self.render_server_list(cx);
        
        div()
            .flex()
            .w_full()
            .h_full()
            .child(server_list)
            .child(
                channels_view_entity.update(cx, |view, cx| {
                    div().child(view.render(window, cx))
                })
            )
            .child(self.render_message_view(cx))
    }
}

