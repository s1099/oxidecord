use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, Pixels,
};
use gpui_component::label::Label;
use gpui_component::button::Button;
use std::sync::{Arc, Mutex};
use crate::app::AppState;

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

        let mut list_container = div().flex().flex_col().gap_1();
        for channel in &channels {
            let channel_id = channel.id;
            let channel_name = channel.name.clone();
            let is_selected = selected.as_ref().map(|id| *id == channel_id).unwrap_or(false);
            
            let channel_id_val = channel_id.get();
            let app_clone = self.app.clone();
            list_container = list_container.child(
                Button::new(("channel", channel_id_val))
                    .on_click(cx.listener(move |_view, _, _, cx| {
                        crate::app::AppState::fetch_messages(app_clone.clone(), channel_id);
                        cx.notify();
                    }))
                    .w_full()
                    .justify_start()
                    .p_2()
                    .px_4()
                    .rounded_md()
                    .bg(if is_selected {
                        gpui::rgb(0x5865f2)
                    } else {
                        gpui::rgb(0x2f3136)
                    })
                    .child(
                        div()
                            .text_color(gpui::rgb(0xdcddde))
                            .child(format!("# {}", channel_name))
                    )
            );
        }

        div()
            .flex()
            .flex_col()
            .w(Pixels::from(240u32))
            .bg(gpui::rgb(0x2f3136))
            .child(
                div()
                    .p_4()
                    .border_b(gpui::px(1.0))
                    .border_color(gpui::rgb(0x202225))
                    .child(Label::new("Channels"))
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .p_2()
                    .child(list_container)
            )
    }
}

