//! The inline edit box that takes a message's text's place while it's being
//! edited, with Discord's "escape to cancel • enter to save" hint under it.

use gpui_kit::component::input::{self, Textarea};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::screens::home::view::composer::surface;
use crate::screens::home::{EDIT_CONTEXT, EditingMessage, HomeScreen, SaveEdit};

impl HomeScreen {
    pub(super) fn render_edit_box(
        &self,
        editing: &EditingMessage,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let message_id = editing.message_id.get();

        let action = |id: &'static str, label: &'static str| {
            div()
                .id((id, message_id))
                .text_color(theme.link)
                .cursor_pointer()
                .hover(|this| this.underline())
                .child(label)
        };

        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            // Enter is bound to `SaveEdit` in this context ahead of the input's newline. Escape
            // isn't
            // rebound: the input lets it through, so it bubbles up here.
            .key_context(EDIT_CONTEXT)
            .on_action(cx.listener(|this, _: &SaveEdit, window, cx| {
                this.save_edit(window, cx);
            }))
            .on_action(cx.listener(|this, _: &input::Escape, window, cx| {
                this.cancel_edit(window, cx);
            }))
            .child(surface(div(), cx).child(Textarea::new(&editing.input).appearance(false)))
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("escape to")
                    .child(action("edit-cancel", "cancel").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.cancel_edit(window, cx);
                        },
                    )))
                    .child("•")
                    .child("enter to")
                    .child(action("edit-save", "save").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.save_edit(window, cx);
                        },
                    ))),
            )
            .into_any_element()
    }
}
