use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, px,
    prelude::*, img, ObjectFit, InteractiveElement,
};
use gpui_component::scroll::ScrollableElement;
use chrono::{Local, TimeZone};
use gpui_component::label::Label;
use gpui_component::avatar::Avatar;
use gpui_component::Sizable;
use gpui_component::StyledExt;
use gpui_component::skeleton::Skeleton;
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use crate::app::{AppState, MessageInfo, AttachmentInfo};
use crate::views::channel_list::ChannelsView;
use crate::views::server_list::ServerListView;

pub struct ChannelView {
    app: Arc<Mutex<AppState>>,
    channels_view: Option<gpui::Entity<ChannelsView>>,
    server_list_view: Option<gpui::Entity<ServerListView>>,
}

impl ChannelView {
    pub fn new(_window: &mut Window, app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
            channels_view: None,
            server_list_view: None,
        }
    }

    fn get_messages(&self) -> Vec<MessageInfo> {
        self.app.lock()
            .map(|app| app.messages.clone())
            .unwrap_or_default()
    }

    fn get_channel_name(&self) -> String {
        self.app.lock()
            .map(|app| {
                app.channels.iter()
                    .find(|ch| app.selected_channel.map(|id| ch.id == id).unwrap_or(false))
                    .map(|ch| format!("# {}", ch.name.clone()))
                    .unwrap_or_else(|| "Select a channel".to_string())
            })
            .unwrap_or_else(|_| "Select a channel".to_string())
    }

    fn has_selected_guild(&self) -> bool {
        self.app.lock()
            .map(|app| app.selected_guild.is_some())
            .unwrap_or(false)
    }

    fn render_message_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let messages = self.get_messages();
        let channel_name = self.get_channel_name();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(
                // Channel header
                div()
                    .flex()
                    .items_center()
                    .h(px(48.))
                    .px_4()
                    .border_b_1()
                    .border_color(gpui::rgb(0x1e1f22))
                    .bg(gpui::rgb(0x313338))
                    .child(
                        Label::new(channel_name)
                            .text_color(gpui::rgb(0xf2f3f5))
                    )
            )
            .child(
                // Messages area
                div()
                    .flex_1()
                    .relative()
                    .min_h(px(0.))
                    .child(
                        if messages.is_empty() {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .text_color(gpui::rgb(0x949ba4))
                                        .child("No messages yet. Select a channel to see messages.")
                                )
                                .into_any_element()
                        } else {
                            div()
                                .flex()
                                .flex_col()
                                .id("messages-list")
                                .overflow_y_scrollbar()
                                .size_full()
                                .py_4()
                                .children(
                                    messages.into_iter().map(|msg| {
                                        render_message(msg)
                                    })
                                )
                                .into_any_element()
                        }
                    )
            )
    }
}

fn render_message(msg: MessageInfo) -> impl IntoElement {
    let avatar = if let Some(url) = &msg.author_avatar_url {
        Avatar::new()
            .src(url.as_str())
            .with_size(px(40.))
    } else {
        Avatar::new()
            .name(&msg.author_name)
            .with_size(px(40.))
    };

    let image_attachments: Vec<AttachmentInfo> = msg.attachments
        .iter()
        .filter(|att| att.is_image())
        .cloned()
        .collect();

    div()
        .w_full()
        .flex()
        .gap_4()
        .px_4()
        .py_1()
        .hover(|s| s.bg(gpui::rgb(0x2e3035)))
        .child(
            // Avatar column
            div()
                .flex_shrink_0()
                .pt_1()
                .child(avatar)
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    // Author name and timestamp row
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(gpui::rgb(0xf2f3f5))
                                .child(msg.author_name.clone())
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(gpui::rgb(0x949ba4))
                                .child(format_timestamp(&msg.timestamp))
                        )
                )
                .when(!msg.content.is_empty(), |this| {
                    this.child(
                        // Message content
                        div()
                            .text_color(gpui::rgb(0xdbdee1))
                            .line_height(px(22.))
                            .child(msg.content.clone())
                    )
                })
                .children(
                    image_attachments.into_iter().map(|attachment| {
                        render_image_attachment(attachment)
                    })
                )
        )
}

fn render_image_attachment(attachment: AttachmentInfo) -> impl IntoElement {
    // Calculate constrained dimensions (max 400x300)
    let max_width = 400.0_f32;
    let max_height = 300.0_f32;
    
    let (display_width, display_height) = if let (Some(w), Some(h)) = (attachment.width, attachment.height) {
        let width = w as f32;
        let height = h as f32;
        let aspect = width / height;
        
        let (final_width, final_height) = if width > max_width {
            (max_width, max_width / aspect)
        } else {
            (width, height)
        };
        
        let final_height = final_height.min(max_height);
        let final_width = if final_height < (max_width / aspect) {
            final_height * aspect
        } else {
            final_width
        };
        
        (final_width, final_height)
    } else {
        // Default dimensions for unknown size
        (300.0, 200.0)
    };

    div()
        .mt_2()
        .w(px(display_width))
        .h(px(display_height))
        .bg(gpui::rgb(0x262626))
        .rounded(px(8.))
        .overflow_hidden()
        .relative()
        .child(
            // Loading skeleton placeholder
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Skeleton::new()
                        .w_full()
                        .h_full()
                        .rounded(px(8.))
                )
        )
        .child(
            img(attachment.url.clone())
                .w_full()
                .h_full()
                .object_fit(ObjectFit::Cover)
                .rounded(px(8.))
        )
}

fn format_timestamp(timestamp: &str) -> String {
    
    if let Ok(secs) = timestamp.parse::<i64>() {
        let msg_time = Local.timestamp_opt(secs, 0).single();
        let now = Local::now();
        
        if let Some(msg_time) = msg_time {
            let msg_date = msg_time.date_naive();
            let today = now.date_naive();
            let yesterday = today.pred_opt().unwrap_or(today);
            
            let time_str = msg_time.format("%-I:%M %p").to_string();
            
            if msg_date == today {
                format!("Today at {}", time_str)
            } else if msg_date == yesterday {
                format!("Yesterday at {}", time_str)
            } else {
                msg_time.format("%m/%d/%Y").to_string()
            }
        } else {
            timestamp.to_string()
        }
    } else {
        timestamp.to_string()
    }
}

impl Render for ChannelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let has_guild = self.has_selected_guild();
        
        if self.channels_view.is_none() {
            self.channels_view = Some(cx.new(|cx| ChannelsView::new(app.clone(), cx)));
        }
        
        if self.server_list_view.is_none() {
            self.server_list_view = Some(cx.new(|cx| ServerListView::new(app.clone(), cx)));
        }
        
        let server_list_view_entity = self.server_list_view.clone().unwrap();
        let channels_view_entity = self.channels_view.clone().unwrap();
        
        let server_list_el = server_list_view_entity.update(cx, |view, cx| {
            div().child(view.render(window, cx))
        });
        
        if has_guild {
            let channels_el = channels_view_entity.update(cx, |view, cx| {
                div()
                    .h_full()
                    .child(view.render(window, cx))
            });
            
            let message_view_el = self.render_message_view(cx);
            
            div()
                .flex()
                .size_full()
                .child(server_list_el)
                .child(channels_el)
                .child(message_view_el)
        } else {
            div()
                .flex()
                .size_full()
                .child(server_list_el)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_4()
                                .text_color(gpui::rgb(0x949ba4))
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("TODO: DM's page here")
                                )
                        )
                )
        }
    }
}
