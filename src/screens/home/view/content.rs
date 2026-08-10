//! The main content pane: the channel header and the scrolling message list.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, divider::Divider, h_flex, skeleton::Skeleton,
    spinner::Spinner, v_flex,
};

use crate::discord::Channel;
use crate::screens::home::channels::channel_icon_path;
use crate::screens::home::{HomeScreen, View};

use super::MESSAGE_PADDING_X;

fn messages_skeleton() -> impl IntoElement {
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
    pub(crate) fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.view == View::DirectMessages {
            return self.render_dm_content(cx);
        }

        if self.loading {
            return messages_skeleton().into_any_element();
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
            return messages_skeleton().into_any_element();
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

    fn render_channel_header(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
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

    pub(super) fn render_messages(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();

        if self.messages_loading {
            return messages_skeleton().into_any_element();
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

        // Route message images through our own cache instead of gpui's global
        // asset cache, which never evicts. Child `img` elements pick this up via
        // the cache stack; clearing it on channel switch bounds image memory to
        // the messages currently on screen.
        image_cache(self.image_cache.clone())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(container.child(messages_list))
            .into_any_element()
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
