//! The message composer: the reply banner, the staged-attachment tray, and the
//! text input, wrapped in one rounded surface, with the typing indicator
//! floating above it, and the autocomplete's suggestions over that.

mod attachments;
mod autocomplete;
mod emoji_picker;
mod reply_banner;
mod typing_indicator;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, Size, StyleSized as _, h_flex,
    input::Textarea, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::home::{COMPOSER_CONTEXT, EmojiTarget, HomeScreen, SendMessage};
use crate::ui::button::Button;
use crate::ui::depth::radius;

impl HomeScreen {
    /// `can_send` is false on channels the user lacks `SEND_MESSAGES` on, where
    /// a notice takes the composer's place.
    pub(super) fn render_message_bar(&self, can_send: bool, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let has_attachments = !self.pending_attachments.is_empty();

        if !can_send {
            return v_flex()
                .w_full()
                .flex_shrink_0()
                .relative()
                .px_2()
                .pb_2()
                .gap_1()
                .children(self.render_typing_indicator(cx))
                .child(
                    // Same metrics as the composer below, so the conversation doesn't
                    // resize between channels. The `input_*` helpers are what `Input`
                    // sizes itself with.
                    surface(h_flex(), cx).pl_1().child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .items_center()
                            .input_size(Size::Medium)
                            .input_text_size(Size::Medium)
                            .h(COMPOSER_LINE_HEIGHT)
                            .text_color(theme.muted_foreground)
                            .child("You do not have permission to send messages in this channel."),
                    ),
                )
                .into_any_element();
        }

        v_flex()
            .w_full()
            .flex_shrink_0()
            .relative()
            .px_2()
            .pb_2()
            .gap_1()
            .children(self.render_typing_indicator(cx))
            .children(self.render_autocomplete(cx))
            .when_some(self.send_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                // The input's own border and focus ring are off in favour of this
                // surface, so banner, tray and input read as one control.
                surface(v_flex(), cx)
                    .when_some(self.replying_to.clone(), |this, target| {
                        this.child(self.render_reply_banner(&target, cx))
                    })
                    .when(has_attachments, |this| {
                        this.child(self.render_attachment_previews(cx))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .pl_1()
                            .child(
                                h_flex()
                                    .h(COMPOSER_LINE_HEIGHT)
                                    .flex_shrink_0()
                                    .items_center()
                                    .child(
                                        Button::new("add-attachment")
                                            .icon(IconName::Plus)
                                            .ghost()
                                            .small()
                                            .tooltip("Add attachment")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.pick_attachments(window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .key_context(COMPOSER_CONTEXT)
                                    .on_action(cx.listener(|this, _: &SendMessage, window, cx| {
                                        // Enter takes a suggestion rather than sending the
                                        // half-typed word.
                                        match this.autocomplete.as_ref() {
                                            Some(autocomplete) => this.accept_suggestion(
                                                autocomplete.selected,
                                                window,
                                                cx,
                                            ),
                                            None => this.send_current_message(window, cx),
                                        }
                                    }))
                                    .on_action(cx.listener(Self::on_autocomplete_previous))
                                    .on_action(cx.listener(Self::on_autocomplete_next))
                                    .on_action(cx.listener(Self::on_autocomplete_accept))
                                    .on_action(cx.listener(Self::on_autocomplete_escape))
                                    .child(
                                        Textarea::new(&self.message_input)
                                            .appearance(false)
                                            // A multi-line input has no fixed height; this pads one
                                            // line out to the height the can't-send notice uses.
                                            .py(COMPOSER_PADDING_Y),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .h(COMPOSER_LINE_HEIGHT)
                                    .flex_shrink_0()
                                    .items_center()
                                    .pr_1()
                                    .child(
                                        Button::new("emoji-picker")
                                            .icon(Icon::default().path("icons/smile.svg"))
                                            .ghost()
                                            .small()
                                            .selected(self.emoji_picker.as_ref().is_some_and(
                                                |picker| picker.target == EmojiTarget::Composer,
                                            ))
                                            .tooltip("Select emoji")
                                            .on_click(cx.listener(
                                                |this, event: &ClickEvent, window, cx| {
                                                    this.toggle_emoji_picker(
                                                        event.position(),
                                                        EmojiTarget::Composer,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// Roomier than a medium input's 32px, like Discord's.
const COMPOSER_LINE_HEIGHT: Pixels = px(44.);

/// Padding around the input that, on top of the editor's own medium inset,
/// centres a medium input's 20px line in `COMPOSER_LINE_HEIGHT`.
const COMPOSER_PADDING_Y: Pixels = px(4.);

/// Flat on purpose: a shadow made the input read as a box on the page. Dark
/// themes wash it with the input colour. The inline edit box wears it too.
pub(super) fn surface(base: Div, cx: &App) -> Div {
    let theme = cx.theme();
    base.w_full()
        .bg(if theme.mode.is_dark() {
            theme.input.opacity(0.3)
        } else {
            theme.background
        })
        .border_1()
        .border_color(theme.input)
        .rounded(radius::CONTROL)
        .overflow_hidden()
}
