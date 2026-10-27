// Release builds skip the console window; debug builds keep it for logging.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod discord;
mod platform;
mod screens;
mod ui;
mod voice;

use std::sync::Arc;

use gpui_kit::component::{Root, input};
use gpui_kit::*;

use crate::screens::app::AppScreen;
use crate::screens::home::{
    AutocompleteAccept, AutocompleteNext, AutocompletePrevious, COMPOSER_CONTEXT, CopySelection,
    EDIT_CONTEXT, EditLastMessage, HOME_CONTEXT, MarkConversationRead, PasteAttachment, SaveEdit,
    SendMessage,
};

fn main() {
    // The app is its own update helper: with this flag it swaps the binary and exits.
    if std::env::args().any(|arg| arg == platform::updater::APPLY_UPDATE_ARG) {
        platform::updater::apply_update();
    }

    // The login webview doesn't work with DirectComposition.
    #[cfg(target_os = "windows")]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }
    let app = gpui_kit::platform::application()
        .with_assets(assets::Assets)
        .with_http_client(Arc::new(platform::http::ReqwestClient::new()));

    app.run(|cx: &mut App| {
        gpui_kit::component::init(cx);
        ui::theme::init(cx);
        platform::updater::init(cx);

        // No context, so these resolve ahead of the text input's own paste/copy;
        // the handlers pass the keystroke on unless there's an image or a list selection.
        cx.bind_keys([
            KeyBinding::new("ctrl-v", PasteAttachment, None),
            KeyBinding::new("cmd-v", PasteAttachment, None),
            KeyBinding::new("ctrl-c", CopySelection, None),
            KeyBinding::new("cmd-c", CopySelection, None),
        ]);

        // Same context depth as the input's own bindings, where the later one
        // wins, so these must come after `gpui_kit::component::init`.
        // Shift-enter falls through to the input's newline.
        let edit_input = format!("{EDIT_CONTEXT} > Input");
        let composer_input = format!("{COMPOSER_CONTEXT} > Input");
        cx.bind_keys([
            KeyBinding::new("enter", SendMessage, Some(&composer_input)),
            KeyBinding::new("enter", SaveEdit, Some(&edit_input)),
            KeyBinding::new(
                "shift-enter",
                input::Enter {
                    secondary: false,
                    shift: true,
                },
                Some(&composer_input),
            ),
            KeyBinding::new(
                "shift-enter",
                input::Enter {
                    secondary: false,
                    shift: true,
                },
                Some(&edit_input),
            ),
            KeyBinding::new("up", EditLastMessage, Some(&composer_input)),
            // After the edit binding, so suggestions get first claim and pass it on.
            KeyBinding::new("up", AutocompletePrevious, Some(&composer_input)),
            KeyBinding::new("down", AutocompleteNext, Some(&composer_input)),
            KeyBinding::new("tab", AutocompleteAccept, Some(&composer_input)),
            // Screen context, not none: a global binding would beat any input or
            // menu that binds escape.
            KeyBinding::new("escape", MarkConversationRead, Some(HOME_CONTEXT)),
        ]);

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                // Only the taskbar name; the app draws its own window controls.
                title: Some("Oxidecord".into()),
                appears_transparent: true,
                ..Default::default()
            }),
            // The bounds are the size restored to on unmaximize.
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

        // Needs the window open for the toast.
        if platform::updater::supported() {
            platform::updater::check(cx);
        }
    });
}
