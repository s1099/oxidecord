//! The pinned-messages popout, opened from the conversation header's pin
//! button.

use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, spinner::Spinner, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::{avatar, bot_tag};
use crate::ui::depth::{self, Level, radius};

use super::super::markdown::MarkdownOptions;
use super::{MEDIA_MAX_WIDTH, Media};

const LIST_HEIGHT: f32 = 480.;
const LIST_GAP: f32 = 8.;
const AVATAR_SIZE: f32 = 32.;
const PIN_PADDING: f32 = 12.;
const CONTENT_INSET: f32 = PIN_PADDING + AVATAR_SIZE + 12.;
/// Media is fixed-size up to [`MEDIA_MAX_WIDTH`] and doesn't shrink to its column, so the
/// popout is as wide as it takes to hold it: borders, list padding, and the pin's padding
/// and avatar gutter.
const WIDTH: f32 =
    MEDIA_MAX_WIDTH + 2. * 1. + 2. * LIST_GAP + 2. * 1. + CONTENT_INSET + PIN_PADDING;

impl HomeScreen {
    /// Over the whole app like the emoji picker: a layer closes it on an outside click.
    pub(in crate::screens::home) fn render_pins_popup(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let popup = self.pins_popup.as_ref()?;
        let theme = cx.theme();

        let body = if popup.loading {
            h_flex()
                .justify_center()
                .py_6()
                .child(Spinner::new().color(theme.muted_foreground))
                .into_any_element()
        } else if let Some(error) = &popup.error {
            div()
                .p_4()
                .text_sm()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element()
        } else if popup.messages.is_empty() {
            v_flex()
                .items_center()
                .gap_2()
                .py_6()
                .px_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(
                    Icon::default()
                        .path("icons/pin.svg")
                        .size_8()
                        .text_color(theme.muted_foreground),
                )
                .child("This conversation doesn't have any pinned messages... yet.")
                .into_any_element()
        } else {
            // A list needs a height to fill, so the popout keeps its full height whatever is
            // pinned.
            let entity = cx.entity();
            v_flex()
                .h(px(LIST_HEIGHT))
                .child(
                    list(popup.list.clone(), move |ix, _window, cx| {
                        entity.update(cx, |this, cx| this.render_pin_item(ix, cx))
                    })
                    .flex_1(),
                )
                .into_any_element()
        };

        let card = v_flex()
            .occlude()
            .w(px(WIDTH))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            // Clicks inside the card aren't for the dismiss layer under it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .h(px(44.))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.muted.opacity(0.4))
                    .child(Icon::default().path("icons/pin.svg").small())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pinned Messages"),
                    ),
            )
            .child(body);

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            // Consumed, so a click on the pin button closes
                            // the popout instead of closing and reopening it.
                            cx.stop_propagation();
                            this.close_pins(cx);
                        }),
                    )
                    .child(
                        anchored()
                            .anchor(Anchor::TopRight)
                            .position(popup.position)
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// The list can't pad or space items, so each row carries the gap above it, and the last
    /// the one below too.
    fn render_pin_item(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let Some(popup) = &self.pins_popup else {
            return div().into_any_element();
        };
        let Some(message) = popup.messages.get(ix) else {
            return div().into_any_element();
        };
        div()
            .px(px(LIST_GAP))
            .pt(px(LIST_GAP))
            .when(ix + 1 == popup.messages.len(), |this| this.pb(px(LIST_GAP)))
            .child(self.render_pin(message, cx))
            .into_any_element()
    }

    fn render_pin(&self, message: &discord::Message, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let media = Media::of(message);
        let author_name = self
            .author_name(message.author_id, &message.author_name)
            .to_owned();

        div()
            .relative()
            .w_full()
            .p(px(PIN_PADDING))
            .pl(px(CONTENT_INSET))
            .rounded(radius::ITEM)
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .absolute()
                    .top(px(PIN_PADDING))
                    .left(px(PIN_PADDING))
                    .child(avatar(
                        author_name.clone(),
                        message.author_avatar_url.clone(),
                        px(AVATAR_SIZE),
                    )),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .when_some(self.role_color(message.author_id), |this, color| {
                                        this.text_color(rgb(color))
                                    })
                                    .child(author_name),
                            )
                            .when(message.author_bot, |this| this.child(bot_tag(cx)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(message.timestamp_label()),
                            ),
                    )
                    .when(!message.content.is_empty(), |this| {
                        this.child(
                            div().w_full().min_w_0().text_sm().child(
                                self.render_markdown(
                                    &message.markdown,
                                    // Apart from the list's, so a spoiler
                                    // revealed here stays hidden there.
                                    MarkdownOptions::new(
                                        format!("pin-{}", message.id),
                                        &message.mentions,
                                    )
                                    .edited(message.edited_label()),
                                    cx,
                                ),
                            ),
                        )
                    })
                    .when_some(message.forward.as_ref(), |this, forward| {
                        this.child(self.render_forward(message.id.get(), forward, cx))
                    })
                    .when(!media.is_empty(), |this| {
                        this.child(self.render_media(message.id.get(), media, cx))
                    }),
            )
    }
}
