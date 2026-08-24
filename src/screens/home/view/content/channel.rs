//! The guild-channel conversation: its header, messages, and composer.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, divider::Divider, v_flex};

use crate::discord::Channel;
use crate::screens::home::HomeScreen;
use crate::screens::home::channels::channel_icon_path;

use super::{header, header_content, message_list, pane};

impl HomeScreen {
    pub(super) fn render_channel_content(&self, cx: &Context<Self>) -> AnyElement {
        let Some(channel) = self.selected_channel_info().cloned() else {
            return pane(message_list::skeleton().into_any_element(), cx);
        };

        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_channel_header(&channel, cx))
            .child(self.render_messages(cx))
            .child(self.render_message_bar(channel.can_send, cx))
            .into_any_element()
    }

    fn render_channel_header(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        header(
            header_content()
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
                    // Show only the first line and let `truncate` elide the rest.
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
                ),
            cx,
        )
    }
}
