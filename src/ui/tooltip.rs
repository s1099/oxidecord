//! Tooltips: small and inverted — the foreground colour as the fill,
//! so a tooltip reads as a label stuck on the UI rather than more of it.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use crate::ui::depth::{Finish, Level, Lit as _, radius};

const HEIGHT: f32 = 26.;

pub struct Tooltip {
    text: SharedString,
}

pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip { text }).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        // Wrapped so the cursor-clearing margin isn't part of the lit box.
        div().child(
            div()
                .m_3()
                .bg(theme.foreground)
                .lit(
                    Level::RaisedStrong,
                    Finish::Matte,
                    px(HEIGHT),
                    radius::ITEM,
                    cx,
                )
                .px_3()
                .py_1p5()
                .font_family(theme.font_family.clone())
                .text_xs()
                .text_color(theme.background)
                .child(self.text.clone()),
        )
    }
}
