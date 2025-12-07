use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, div, Pixels, px, size, Size,
};
use gpui_component::label::Label;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    v_virtual_list, VirtualListScrollHandle,
    scroll::{Scrollbar, ScrollbarState, ScrollbarAxis},
};
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use crate::app::AppState;
use crate::services::discord::DiscordService;

pub struct ChannelsView {
    app: Arc<Mutex<AppState>>,
    scroll_handle: VirtualListScrollHandle,
    scroll_state: ScrollbarState,
}

impl ChannelsView {
    pub fn new(app: Arc<Mutex<AppState>>, _cx: &mut Context<Self>) -> Self {
        Self {
            app,
            scroll_handle: VirtualListScrollHandle::new(),
            scroll_state: ScrollbarState::default(),
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
        let channel_count = channels.len();
        
        let item_sizes = Rc::new(
            (0..channel_count)
                .map(|_| size(px(240.), px(42.)))
                .collect::<Vec<Size<Pixels>>>()
        );

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
                    .flex_1()
                    .relative()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(
                        v_virtual_list(
                            cx.entity().clone(),
                            "channels-list",
                            item_sizes.clone(),
                            move |view, visible_range, _, cx| {
                                let channels = view.get_channels();
                                let selected = view.get_selected_channel();
                                
                                visible_range
                                    .map(|ix| {
                                        let channel = &channels[ix];
                                        let channel_id = channel.id;
                                        let channel_name = channel.name.clone();
                                        let is_selected = selected.as_ref().map(|id| *id == channel_id).unwrap_or(false);
                                        
                                        let channel_id_val = channel_id.get();
                                        let app_clone = view.app.clone();
                                        
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
                                    .collect()
                            },
                        )
                        .track_scroll(&self.scroll_handle)
                        .p_2()
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .child(
                                Scrollbar::both(&self.scroll_state, &self.scroll_handle)
                                    .axis(ScrollbarAxis::Vertical)
                            )
                    )
            )
    }
}
