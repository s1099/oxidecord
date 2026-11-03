//! The main window's router.

use gpui_kit::*;

use crate::discord;
use crate::screens::home::{AddAccount, HomeScreen, SessionEnded, SwitchAccount};
use crate::screens::login::{self, LoginScreen};
use crate::ui::{dialogs, update_notice};

enum Route {
    Login(Entity<LoginScreen>),
    /// `_events` send the user back to login when the session ends, and carry out account
    /// switches.
    Home {
        screen: Entity<HomeScreen>,
        _events: [Subscription; 3],
    },
}

pub struct AppScreen {
    route: Route,
    _update_notice: Subscription,
}

impl AppScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let route = if discord::load_token().is_some() {
            Self::home_route(window, cx)
        } else {
            Self::login_route(window, cx)
        };

        Self {
            route,
            _update_notice: update_notice::watch(window, cx),
        }
    }

    /// Opens the active account, replacing any account already open.
    pub fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Route::Home { screen, .. } = &self.route {
            screen.update(cx, |home, cx| home.end_session(cx));
        }
        self.route = Self::home_route(window, cx);
        cx.notify();
    }

    fn home_route(window: &mut Window, cx: &mut Context<Self>) -> Route {
        let home = cx.new(|cx| HomeScreen::new(window, cx));
        let ended = cx.subscribe_in(
            &home,
            window,
            |this, _home, _: &SessionEnded, window, cx| {
                this.route = Self::login_route(window, cx);
                cx.notify();
            },
        );
        let switch = cx.subscribe_in(
            &home,
            window,
            |this, _home, SwitchAccount(id): &SwitchAccount, window, cx| {
                match discord::switch_account(*id) {
                    Ok(()) => this.show_home(window, cx),
                    Err(err) => dialogs::show_error(
                        "Couldn't switch accounts",
                        [err.to_string()],
                        window,
                        cx,
                    ),
                }
            },
        );
        let add = cx.subscribe_in(&home, window, |_this, _home, _: &AddAccount, window, cx| {
            login::open_login_window(cx.weak_entity(), window.window_handle(), cx);
        });
        Route::Home {
            screen: home,
            _events: [ended, switch, add],
        }
    }

    fn login_route(window: &mut Window, cx: &mut Context<Self>) -> Route {
        let app = cx.weak_entity();
        Route::Login(cx.new(|cx| LoginScreen::new(app, window, cx)))
    }
}

impl Render for AppScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = match &self.route {
            Route::Login(view) => view.clone().into_any_element(),
            Route::Home { screen: view, .. } => view.clone().into_any_element(),
        };

        // Toasts draw above dialogs since one can matter while a dialog is up.
        dialogs::with_dialog_layer(screen, window, cx)
    }
}
