mod views;

use gpui::*;
use gpui_component::Root;
use views::login::LoginView;

use crate::views::servers::ServersView;

fn main() {
    let app = Application::new();

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                // let view = cx.new(|cx| LoginView::new(window, cx));
                let view = cx.new(|cx| ServersView::new(window, cx));

                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}