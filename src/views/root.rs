use gpui::*;
use crate::app::{AppState, View};
use crate::views::login::LoginView;
use crate::views::channel_messages::ChannelView;
use std::sync::{Arc, Mutex};

pub struct RootView {
    app: Arc<Mutex<AppState>>,
    login_view: Entity<LoginView>,
    channel_view: Option<Entity<ChannelView>>,
}

impl RootView {
    pub fn new(window: &mut Window, app: Arc<Mutex<AppState>>, cx: &mut Context<Self>) -> Self {
        let login_view = cx.new(|cx| LoginView::new(window, app.clone(), cx));
        Self {
            app,
            login_view,
            channel_view: None,
        }
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_view = self.app.lock().map(|app| app.current_view.clone()).unwrap_or(View::Login);

        match current_view {
            View::Login => self.login_view.update(cx, |view, cx| {
                div().child(view.render(window, cx))
            }),
            View::Servers | View::Channel => {
                let channel_view = self.channel_view.get_or_insert_with(|| {
                    cx.new(|cx| ChannelView::new(window, self.app.clone(), cx))
                });
                channel_view.update(cx, |view, cx| {
                    div()
                    .flex()
                    .h_full()
                    .child(view.render(window, cx))
                })
            }
        }
    }
}
