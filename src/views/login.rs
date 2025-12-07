use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, Entity, prelude::*};
use gpui_component::button::*;
use gpui_component::form::{field, v_form};
use gpui_component::input::{Input, InputState};
use gpui_component::Disableable;
use std::sync::{Arc, Mutex};
use crate::app::AppState;
use crate::services::discord::DiscordService;

pub struct LoginView {
    token: Entity<InputState>,
    app: Arc<Mutex<AppState>>,
}

impl LoginView {
    pub fn new(window: &mut Window, app: Arc<Mutex<AppState>>, cx: &mut Context<Self>) -> Self {
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Token")
        });
        Self { token, app }
    }

    pub fn login(&mut self, cx: &mut Context<Self>) {
        let token = self.token.read(cx).value();
        if !token.is_empty() {
            DiscordService::login(self.app.clone(), token.to_string());
            cx.notify();
        }
    }
}

impl Render for LoginView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.app.lock().map(|app| app.loading).unwrap_or(false);
        let error = self.app.lock().map(|app| app.error.clone()).unwrap_or(None);
        
        let mut form = v_form()
            .child(
                field()
                    .label("Discord Token")
                    .child(Input::new(&self.token)),
            )
            .child(
                field().label_indent(false).child(
                    Button::new("login")
                        .primary()
                        .disabled(loading)
                        .child(if loading { "Logging in..." } else { "Login" })
                        .on_click(cx.listener(|this, _, _, cx| this.login(cx))),
                ),
            );
        
        if let Some(error_msg) = error {
            form = form.child(
                field().label_indent(false).child(
                    div()
                        .text_color(gpui::red())
                        .text_sm()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .child(error_msg)
                )
            );
        }
        
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .child(
                div().w_112().child(form)
            )
    }
}
