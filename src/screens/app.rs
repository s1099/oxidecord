use gpui::*;

use crate::screens::dialogs;
use crate::screens::home::HomeScreen;
use crate::screens::login::LoginScreen;

/// Which screen the app is currently showing.
enum Route {
    Login(Entity<LoginScreen>),
    Home(Entity<HomeScreen>),
}

/// Top-level view for the main window. Owns the current screen and swaps
/// between login and home so that a successful login can take effect
/// immediately, without restarting the app.
pub struct AppScreen {
    route: Route,
}

impl AppScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let route = if crate::discord::load_token().is_some() {
            Route::Home(cx.new(|cx| HomeScreen::new(window, cx)))
        } else {
            let app = cx.weak_entity();
            Route::Login(cx.new(|cx| LoginScreen::new(app, window, cx)))
        };

        Self { route }
    }

    /// Switch to the home screen once a token is available.
    pub fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let home = cx.new(|cx| HomeScreen::new(window, cx));
        self.route = Route::Home(home);
        cx.notify();
    }
}

impl Render for AppScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = match &self.route {
            Route::Login(view) => view.clone().into_any_element(),
            Route::Home(view) => view.clone().into_any_element(),
        };

        // Every screen sits under the same modal layer, so a dialog opened
        // anywhere in the app is drawn (and backed by its scrim) here.
        dialogs::with_dialog_layer(screen, window, cx)
    }
}
