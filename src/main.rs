mod app;
mod views;
mod services;
mod utils;

use gpui::*;
use gpui_component::Root;
use app::AppState;
use views::root::RootView;
use std::sync::{Arc, Mutex};

fn main() {
    utils::init_runtime();
    
    let app = Application::new();

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            let app_state = Arc::new(Mutex::new(AppState::new()));
            
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| RootView::new(window, app_state, cx));

                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
