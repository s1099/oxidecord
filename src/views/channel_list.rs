use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, Pixels, px,
};
use gpui_component::label::Label;
use gpui_component::button::{Button, ButtonVariants};
use gpui::InteractiveElement;
use gpui_component::scroll::ScrollableElement;
use std::sync::{Arc, Mutex};
use crate::app::AppState;
use crate::services::discord::DiscordService;

pub struct ChannelsView {
    app: Arc<Mutex<AppState>>,
}

impl ChannelsView {
    pub fn new(app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
        }
    }

    fn get_channels(&self) -> Vec<crate::app::ChannelInfo> {
        self.app.lock()
            .map(|app| app.channels.clone())
            .unwrap_or_default()
    }

    fn get_selected_channel(&self) -> Option<twilight_model::id::Id<twilight_model::id::marker::ChannelMarker>> {
        self.app.lock()
            .map(|app| app.selected_channel.clone())
            .unwrap_or(None)
    }
}

impl Render for ChannelsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let channels = self.get_channels();
        let selected = self.get_selected_channel();

        div()
            .flex()
            .flex_col()
            .w(Pixels::from(240u32))
            .h_full()
            .child(
                div()
                    .p_4()
                    .border_b_1()
                    .child(Label::new("Channels"))
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .id("channels-list")
                    .overflow_y_scrollbar()
                    .flex_1()
                    .min_h(px(0.))
                    .p_2()
                    .children(
                        channels.into_iter().map(|channel| {
                            let channel_id = channel.id;
                            let channel_name = channel.name.clone();
                            let is_selected = selected.as_ref().map(|id| *id == channel_id).unwrap_or(false);
                            
                            let channel_id_val = channel_id.get();
                            let app_clone = self.app.clone();
                            
                            let mut button = Button::new(("channel", channel_id_val))
                                .on_click(cx.listener(move |_view, _, _, cx| {
                                    DiscordService::fetch_messages(app_clone.clone(), channel_id);
                                    cx.notify();
                                }))
                                .w_full()
                                .justify_start()
                                .p_2()
                                .px_4()
                                .rounded_md();
                            
                            if is_selected {
                                button = button.primary();
                            }

                            button.child(
                                div()
                                    .child(format!("# {}", channel_name))
                            )
                        })
                    )
            )
    }
}
