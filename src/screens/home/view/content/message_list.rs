//! The scrolling message list, and the skeleton shown in its place while a
//! conversation loads.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, h_flex, skeleton::Skeleton, spinner::Spinner, v_flex,
};

use crate::screens::home::HomeScreen;

use crate::screens::home::view::MESSAGE_PADDING_X;

/// Placeholder message rows. The widths are fixed rather than random so the
/// skeleton doesn't reshuffle on every frame.
pub(super) fn skeleton() -> impl IntoElement {
    const WIDTHS: [f32; 12] = [
        420., 280., 360., 200., 480., 320., 260., 440., 300., 380., 220., 460.,
    ];
    v_flex()
        .flex_1()
        .w_full()
        .justify_end()
        .gap_4()
        .py_2()
        .children(WIDTHS.iter().map(|&content_width| {
            h_flex()
                .w_full()
                .px(px(MESSAGE_PADDING_X))
                .gap_3()
                .items_start()
                .child(Skeleton::new().size(px(40.)).rounded_full())
                .child(
                    v_flex()
                        .flex_1()
                        .gap_2()
                        .child(Skeleton::new().w(px(120.)).h_4().rounded_md())
                        .child(Skeleton::new().w(px(content_width)).h_4().rounded_md()),
                )
        }))
}

impl HomeScreen {
    pub(super) fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return skeleton().into_any_element();
        }

        if let Some(error) = &self.messages_error {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element();
        }

        let entity = cx.entity();
        let messages_list = list(self.messages_list.clone(), move |ix, _window, cx| {
            entity.update(cx, |this, cx| this.render_message_item(ix, cx))
        })
        .flex_1()
        .py_2();

        let mut container = v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.messages_scroll.absorb()));
        if self.older_loading {
            container = container.child(
                h_flex()
                    .w_full()
                    .py_1()
                    .justify_center()
                    .child(Spinner::new().small().color(theme.muted_foreground)),
            );
        }

        // Note: images inside the list name `self.image_cache` on the element
        // itself rather than relying on an ancestor `image_cache(..)`. That
        // wrapper only pushes onto the cache stack during layout and paint,
        // while `list` renders its items during prepaint, so the stack would be
        // empty when the images actually resolve and they'd land in gpui's
        // global asset cache, which never evicts.
        container.child(messages_list).into_any_element()
    }

    fn render_message_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        // Consecutive messages from the same author share one header, except
        // replies always show theirs so the quoted preview has room to sit
        // above the message, like Discord.
        let show_header = ix == 0
            || message.reply.is_some()
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        // Mirrors the `show_header` logic applied to `ix + 1`.
        let next_starts_group = self
            .messages
            .get(ix + 1)
            .is_some_and(|next| next.reply.is_some() || next.author_id != message.author_id);
        self.render_message(message, show_header, next_starts_group, cx)
    }
}
