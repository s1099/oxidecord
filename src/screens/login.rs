use gpui::*;
use gpui_component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    tab::{Tab, TabBar},
    v_flex,
};

use crate::screens::app::AppScreen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginMethod {
    Discord,
    Token,
}

pub struct LoginScreen {
    app: WeakEntity<AppScreen>,
    method: LoginMethod,
    token_input: Entity<InputState>,
}

impl LoginScreen {
    pub fn new(app: WeakEntity<AppScreen>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste your token")
                .masked(true)
        });

        Self {
            app,
            method: LoginMethod::Discord,
            token_input,
        }
    }

    fn render_logo(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex().items_center().gap(px(4.)).child(
            div()
                .text_size(px(36.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(theme.primary)
                .child("Oxidecord"),
        )
    }

    fn render_method_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_index = match self.method {
            LoginMethod::Discord => 0,
            LoginMethod::Token => 1,
        };

        let entity = cx.entity();
        TabBar::new("login-tabs")
            .segmented()
            .selected_index(active_index)
            .child(Tab::new().label("Login with Discord"))
            .child(Tab::new().label("Login with Token"))
            .on_click(move |&selected_index, _window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.method = match selected_index {
                        0 => LoginMethod::Discord,
                        _ => LoginMethod::Token,
                    };
                    cx.notify();
                });
            })
    }

    fn render_discord_pane(&self) -> impl IntoElement {
        let app = self.app.clone();

        v_flex().w_full().gap(px(16.)).items_center().child(
            Button::new("btn-discord-login")
                .label("Continue with Discord")
                .primary()
                .rounded(px(8.))
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
                        window_bounds: Some(WindowBounds::centered(size(px(500.), px(650.)), cx)),
                        window_min_size: Some(size(px(400.), px(520.))),
                        ..Default::default()
                    };

                    cx.open_window(webview_options, move |window, cx| {
                        let webview_view = cx.new(|cx| {
                            crate::screens::login_webview::LoginWebview::new(
                                app,
                                main_window,
                                window,
                                cx,
                            )
                        });
                        cx.new(|cx| gpui_component::Root::new(webview_view, window, cx))
                    })
                    .expect("Failed to open webview window");
                }),
        )
    }

    fn render_token_pane(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap(px(16.))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("DISCORD TOKEN"),
                    )
                    .child(Input::new(&self.token_input).mask_toggle().cleanable(true)),
            )
            .child(
                Button::new("btn-token-login")
                    .label("Log In")
                    .primary()
                    .rounded(px(8.)),
            )
    }
}

impl Render for LoginScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let card_bg = cx.theme().secondary;
        let page_bg = cx.theme().background;

        let card = v_flex()
            .w(px(400.))
            .p(px(32.))
            .gap(px(24.))
            .rounded(px(8.))
            .bg(card_bg)
            .shadow_lg()
            .child(self.render_logo(cx))
            .child(v_flex().items_center().child(self.render_method_tabs(cx)))
            .child(match self.method {
                LoginMethod::Discord => self.render_discord_pane().into_any_element(),
                LoginMethod::Token => self.render_token_pane(cx).into_any_element(),
            });

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(page_bg)
            .child(card)
    }
}
