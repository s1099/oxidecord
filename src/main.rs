mod app;
mod views;

use gpui::*;
use gpui_component::Root;
use app::{AppState, View};
use views::login::LoginView;
use views::channel::ChannelView;
use std::sync::{Arc, Mutex};

struct RootView {
    app: Arc<Mutex<AppState>>,
    login_view: Entity<LoginView>,
    channel_view: Option<Entity<ChannelView>>,
}

impl RootView {
    fn new(window: &mut Window, app: Arc<Mutex<AppState>>, cx: &mut Context<Self>) -> Self {
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
        // Observe the app state to re-render when view changes
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
                    div().child(view.render(window, cx))
                })
            }
        }
    }
}

fn main() {
    // Initialize Tokio runtime handle before starting GPUI
    AppState::init_runtime();
    
    let app = Application::new();

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            let app_state = Arc::new(Mutex::new(AppState::new()));
            
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| RootView::new(window, app_state, cx));

                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
