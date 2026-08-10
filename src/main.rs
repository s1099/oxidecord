mod assets;
mod constants;
mod discord;
mod http;
mod runtime;
mod screens;
mod smooth_scroll;

use std::sync::Arc;

use gpui::*;
use gpui_component::Root;

use crate::screens::app::AppScreen;

fn main() {
    // required for webview to work on windows
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }
    let app = Application::new()
        .with_assets(assets::Assets)
        .with_http_client(Arc::new(http::ReqwestClient::new()));

    app.run(|cx: &mut App| {
        gpui_component::init(cx);

        // Bound with no context so it resolves ahead of the text input's own
        // paste binding; the handler consumes the event only when an image is
        // pasted into the composer and otherwise lets text paste through.
        cx.bind_keys([
            KeyBinding::new("ctrl-v", crate::screens::home::PasteAttachment, None),
            KeyBinding::new("cmd-v", crate::screens::home::PasteAttachment, None),
        ]);

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Oxidecord".into()),
                ..Default::default()
            }),
            // Open maximized; the bounds are the size to restore to when unmaximized.
            window_bounds: Some(WindowBounds::Maximized(Bounds::centered(
                None,
                size(px(1200.), px(800.)),
                cx,
            ))),
            window_min_size: Some(size(px(480.), px(520.))),
            ..Default::default()
        };

        cx.open_window(options, move |window, cx| {
            let app_view = cx.new(|cx| AppScreen::new(window, cx));
            cx.new(|cx| Root::new(app_view, window, cx))
        })
        .expect("Failed to open window");
    });
}
