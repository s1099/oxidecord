use gpui::{AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::button::*;
use gpui_component::form::{field, v_form};
use gpui_component::input::{Input, InputState};
pub struct LoginView {
    token: Entity<InputState>,
}

impl LoginView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Token")
                .masked(true)
        });
        Self { token }
    }

    pub fn login(&mut self, cx: &mut Context<Self>) {
        let token = self.token.read(cx).value();
        println!("Token: {:?}", token);
    }
}

impl Render for LoginView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w_112().child(
            v_form()
                .child(
                    field()
                        .label("Token")
                        .child(Input::new(&self.token).mask_toggle()),
                )
                .child(
                    field().label_indent(false).child(
                        Button::new("login")
                            .primary()
                            .child("Login")
                            .on_click(cx.listener(|this, _, _, cx| this.login(cx))),
                    ),
                ),
        )
    }
}
