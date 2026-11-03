//! The sign-in screen: pick a saved account, or sign in on Discord's page and hand the app
//! a token.

mod webview;

use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
// Token login is hidden for now; its imports come back with it.
use gpui_kit::component::{
    ActiveTheme as _,
    IconName,
    // input::{Input, InputState},
    // tab::{Tab, TabBar},
    Sizable as _,
    h_flex,
    v_flex,
};
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, Account};
use crate::screens::app::AppScreen;
use crate::ui::accounts::{account_avatar, account_label};
use crate::ui::button::Button;
use crate::ui::depth::{self, radius};
use crate::ui::window_controls::WindowControls;

use webview::LoginWebview;

// Token login is hidden: only "Login with Discord" is offered. The method
// selector and token pane are kept commented out so they can be restored.
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// enum LoginMethod {
//     Discord,
//     Token,
// }

/// `logo.png` is pre-shrunk to twice this for high-DPI: gpui has no mipmaps,
/// so drawing the full 1024px icon this small aliases.
const LOGO_SIZE: f32 = 72.;

/// Opens Discord's login page in its own window. A token it reports becomes the active
/// account and `main_window` switches to it.
pub fn open_login_window(app: WeakEntity<AppScreen>, main_window: AnyWindowHandle, cx: &mut App) {
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Discord Login".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::centered(size(px(875.), px(665.)), cx)),
        window_min_size: Some(size(px(400.), px(520.))),
        ..Default::default()
    };

    if let Err(err) = cx.open_window(options, move |window, cx| {
        let webview = cx.new(|cx| LoginWebview::new(app, main_window, window, cx));
        cx.new(|cx| gpui_kit::component::Root::new(webview, window, cx))
    }) {
        eprintln!("failed to open the login window: {err}");
    }
}

pub struct LoginScreen {
    app: WeakEntity<AppScreen>,
    logo: Arc<Image>,
    /// Saved sign-ins to pick from; with none, the plain welcome.
    accounts: Vec<Account>,
    error: Option<SharedString>,
    // method: LoginMethod,
    // token_input: Entity<InputState>,
}

impl LoginScreen {
    pub fn new(app: WeakEntity<AppScreen>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        // let token_input = cx.new(|cx| {
        //     InputState::new(window, cx)
        //         .placeholder("Paste your token")
        //         .masked(true)
        // });

        Self {
            app,
            logo: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../assets/logo.png").to_vec(),
            )),
            accounts: discord::accounts(),
            error: None,
            // method: LoginMethod::Discord,
            // token_input,
        }
    }

    fn switch_to(&mut self, id: Id<UserMarker>, window: &mut Window, cx: &mut Context<Self>) {
        match discord::switch_account(id) {
            Ok(()) => {
                let _ = self.app.update(cx, |app, cx| app.show_home(window, cx));
            }
            Err(err) => {
                self.accounts = discord::accounts();
                self.error = Some(err.to_string().into());
                cx.notify();
            }
        }
    }

    fn render_logo(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title = if self.accounts.is_empty() {
            "Welcome to Oxidecord"
        } else {
            "Choose an account"
        };

        v_flex()
            .items_center()
            .gap(px(6.))
            .child(img(self.logo.clone()).size(px(LOGO_SIZE)).mb(px(10.)))
            .child(
                div()
                    .text_size(px(26.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
    }

    fn render_accounts(&self, cx: &Context<Self>) -> impl IntoElement {
        let hover = cx.theme().secondary_hover;
        v_flex()
            .w_full()
            .gap_1()
            .children(self.accounts.iter().map(|account| {
                let id = account.id;
                h_flex()
                    .id(SharedString::from(format!("account-{id}")))
                    .w_full()
                    .gap_3()
                    .p_2()
                    .items_center()
                    .rounded(radius::ITEM)
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover))
                    .child(account_avatar(account, px(40.)))
                    .child(account_label(account, cx))
                    .child(
                        Button::new(SharedString::from(format!("remove-account-{id}")))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .tooltip("Log out of this account")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // The row underneath would switch to it otherwise.
                                cx.stop_propagation();
                                discord::remove_account(id);
                                this.accounts = discord::accounts();
                                this.error = None;
                                cx.notify();
                            })),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_to(id, window, cx)),
                    )
            }))
    }

    // fn render_method_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
    //     let active_index = match self.method {
    //         LoginMethod::Discord => 0,
    //         LoginMethod::Token => 1,
    //     };
    //
    //     let entity = cx.entity();
    //     TabBar::new("login-tabs")
    //         .segmented()
    //         .selected_index(active_index)
    //         .child(Tab::new().label("Login with Discord"))
    //         .child(Tab::new().label("Login with Token"))
    //         .on_click(move |&selected_index, _window, cx| {
    //             entity.update(cx, |this, cx| {
    //                 this.method = match selected_index {
    //                     0 => LoginMethod::Discord,
    //                     _ => LoginMethod::Token,
    //                 };
    //                 cx.notify();
    //             });
    //         })
    // }

    fn render_discord_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        let app = self.app.clone();

        let button = Button::new("btn-discord-login").large().w_full().on_click(
            move |_event, window, cx| {
                // The button lives in the main window, which switches to Home once the
                // webview reports a token.
                open_login_window(app.clone(), window.window_handle(), cx);
            },
        );
        let button = if self.accounts.is_empty() {
            button.label("Continue with Discord").primary()
        } else {
            button
                .icon(IconName::Plus)
                .label("Add an account")
                .outline()
        };

        v_flex()
            .w_full()
            .gap(px(12.))
            .items_center()
            .child(button)
            .child(
                div()
                    .text_size(px(12.))
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child("You'll sign in on Discord's own page."),
            )
    }

    // fn render_token_pane(&self, cx: &Context<Self>) -> impl IntoElement {
    //     v_flex()
    //         .w_full()
    //         .gap(px(16.))
    //         .child(
    //             v_flex()
    //                 .gap(px(6.))
    //                 .child(
    //                     div()
    //                         .text_size(px(12.))
    //                         .font_weight(FontWeight::SEMIBOLD)
    //                         .text_color(cx.theme().muted_foreground)
    //                         .child("DISCORD TOKEN"),
    //                 )
    //                 .child(Input::new(&self.token_input).mask_toggle().cleanable(true)),
    //         )
    //         .child(Button::new("btn-token-login").label("Log In").primary())
    // }
}

impl Render for LoginScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page_bg = cx.theme().sidebar;

        let card = depth::card(radius::CARD, cx)
            .flex()
            .flex_col()
            .w(px(380.))
            .px(px(32.))
            .pt(px(36.))
            .pb(px(28.))
            .gap(px(28.))
            .child(self.render_logo(cx))
            .when(!self.accounts.is_empty(), |this| {
                this.child(self.render_accounts(cx))
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            // .child(v_flex().items_center().child(self.render_method_tabs(cx)))
            // .child(match self.method {
            //     LoginMethod::Discord => self.render_discord_pane().into_any_element(),
            //     LoginMethod::Token => self.render_token_pane(cx).into_any_element(),
            // });
            .child(self.render_discord_pane(cx));

        v_flex()
            .size_full()
            .bg(page_bg)
            // No system title bar and no header, so the controls get their own
            // strip, which also drags the window.
            .child(
                h_flex()
                    .h(px(40.))
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .child(
                        div()
                            .id("login-drag")
                            .flex_1()
                            .h_full()
                            .when(cfg!(target_os = "windows"), |this| {
                                this.window_control_area(WindowControlArea::Drag)
                            }),
                    )
                    .child(WindowControls::default()),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
    }
}
