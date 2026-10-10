//! The sign-in screen: pick a login method, then hand the app a token.

mod webview;

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
// Token login is hidden for now; its imports come back with it.
use gpui_component::{
    ActiveTheme as _,
    // input::{Input, InputState},
    // tab::{Tab, TabBar},
    Sizable as _,
    h_flex,
    v_flex,
};

use crate::screens::app::AppScreen;
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

/// Drawn size of the app icon at the top of the card. `logo.png` is the app
/// icon shrunk ahead of time to twice this, for high-DPI screens: gpui scales
/// images without mipmaps, so drawing the full 1024px icon this small aliases.
const LOGO_SIZE: f32 = 72.;

pub struct LoginScreen {
    app: WeakEntity<AppScreen>,
    logo: Arc<Image>,
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
            // method: LoginMethod::Discord,
            // token_input,
        }
    }

    fn render_logo(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .items_center()
            .gap(px(6.))
            .child(img(self.logo.clone()).size(px(LOGO_SIZE)).mb(px(10.)))
            .child(
                div()
                    .text_size(px(26.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.foreground)
                    .child("Welcome to Oxidecord"),
            )
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

        v_flex()
            .w_full()
            .gap(px(12.))
            .items_center()
            .child(
                Button::new("btn-discord-login")
                    .label("Continue with Discord")
                    .primary()
                    .large()
                    .w_full()
                    .on_click(move |_event, window, cx| {
                        // The button renders in the main window, so this handle
                        // points at the window we want to switch to Home once the
                        // login webview reports a token.
                        let main_window = window.window_handle();
                        let app = app.clone();

                        let webview_options = WindowOptions {
                            titlebar: Some(TitlebarOptions {
                                title: Some("Discord Login".into()),
                                ..Default::default()
                            }),
                            window_bounds: Some(WindowBounds::centered(
                                size(px(500.), px(650.)),
                                cx,
                            )),
                            window_min_size: Some(size(px(400.), px(520.))),
                            ..Default::default()
                        };

                        cx.open_window(webview_options, move |window, cx| {
                            let webview_view =
                                cx.new(|cx| LoginWebview::new(app, main_window, window, cx));
                            cx.new(|cx| gpui_component::Root::new(webview_view, window, cx))
                        })
                        .expect("Failed to open webview window");
                    }),
            )
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
        // The page is the sidebar colour, so the card lifts off it the way the
        // conversation panel does on the home screen.
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
            // .child(v_flex().items_center().child(self.render_method_tabs(cx)))
            // .child(match self.method {
            //     LoginMethod::Discord => self.render_discord_pane().into_any_element(),
            //     LoginMethod::Token => self.render_token_pane(cx).into_any_element(),
            // });
            .child(self.render_discord_pane(cx));

        v_flex()
            .size_full()
            .bg(page_bg)
            // No system title bar, so the window's own controls have to be on
            // the page. Login has no header to put them in, so they get a strip
            // of their own; the rest of it drags the window.
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
