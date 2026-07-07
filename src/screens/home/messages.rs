use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    avatar::Avatar, divider::Divider, h_flex, input::Input, v_flex, ActiveTheme as _, Icon,
    Sizable as _,
};

use crate::discord::{self, Channel};

use super::channels::channel_icon_path;
use super::HomeScreen;

/// Horizontal padding, in pixels, on either side of the message list. Applied
/// per message rather than on the list, whose padding its items overflow.
const MESSAGE_PADDING_X: f32 = 16.;

impl HomeScreen {
    pub(super) fn render_channel_header(
        &self,
        channel: &Channel,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();

        h_flex()
            .h(px(48.))
            .w_full()
            .flex_shrink_0()
            .px_4()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path(channel_icon_path(channel.kind))
                    .size_5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(channel.name.clone()),
            )
            .when_some(
                // show only the first line and let `truncate` elide the rest.
                channel
                    .topic
                    .as_deref()
                    .and_then(|topic| topic.lines().find(|line| !line.trim().is_empty()))
                    .map(str::to_owned),
                |this, topic| {
                    this.child(Divider::vertical().h(px(24.))).child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(topic),
                    )
                },
            )
    }

    fn render_message(
        &self,
        message: &discord::Message,
        show_header: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let content: AnyElement = if message.content.is_empty() {
            div()
                .italic()
                .text_color(theme.muted_foreground)
                .child("(no text content)")
                .into_any_element()
        } else {
            div()
                .w_full()
                .min_w_0()
                .child(message.content.clone())
                .into_any_element()
        };

        // Consecutive messages from the same author share one header, like
        // Discord; align follow-ups with the content column (avatar + gap).
        // The list itself can't pad its items (they overflow its padding), so
        // each message carries its own horizontal padding and a full width with
        // `min_w_0` so long lines wrap instead of running off the right edge.
        if !show_header {
            return div()
                .id(("message", message.id.get()))
                .w_full()
                .min_w_0()
                .pl(px(MESSAGE_PADDING_X + 52.))
                .pr(px(MESSAGE_PADDING_X))
                .py(px(1.))
                .text_sm()
                .child(content)
                .into_any_element();
        }

        let mut avatar = Avatar::new()
            .name(message.author_name.clone())
            .with_size(px(40.));
        if let Some(avatar_url) = message.author_avatar_url.clone() {
            avatar = avatar.src(avatar_url);
        }

        h_flex()
            .id(("message", message.id.get()))
            .w_full()
            .pt(px(16.))
            .px(px(MESSAGE_PADDING_X))
            .gap_3()
            .items_start()
            .child(avatar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(message.author_name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(message.timestamp.clone()),
                            ),
                    )
                    .child(div().w_full().min_w_0().text_sm().child(content)),
            )
            .into_any_element()
    }

    pub(super) fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Loading messages...")
                .into_any_element();
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

        let mut container = v_flex().flex_1().min_h_0().w_full();
        if self.older_loading {
            container = container.child(
                h_flex()
                    .w_full()
                    .py_1()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Loading older messages..."),
            );
        }
        container.child(messages_list).into_any_element()
    }

    fn render_message_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.messages.get(ix) else {
            return div().into_any_element();
        };
        // Consecutive messages from the same author share one header.
        let show_header = ix == 0
            || self.messages.get(ix - 1).map(|previous| previous.author_id)
                != Some(message.author_id);
        self.render_message(message, show_header, cx)
    }

    pub(super) fn render_message_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .w_full()
            .flex_shrink_0()
            .px_4()
            .pt_1()
            .pb_4()
            .gap_1()
            .when_some(self.send_error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(Input::new(&self.message_input))
    }

    pub(super) fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Loading...")
                .into_any_element();
        }

        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(theme.danger).child(error.clone()))
                .into_any_element();
        }

        let Some(channel) = self.selected_channel_info().cloned() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select a channel")
                .into_any_element();
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_channel_header(&channel, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(cx))
            .into_any_element()
    }
}
