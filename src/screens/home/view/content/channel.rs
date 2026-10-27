//! The guild-channel conversation: its header, messages, and composer.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, separator::Separator,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::Channel;
use crate::screens::home::HomeScreen;
use crate::screens::home::channels::channel_icon_path;
use crate::ui::button::Button;
use crate::ui::theme;

use super::super::markdown::MarkdownOptions;
use super::{header, header_actions, header_content, message_list, pane};

/// Share of the pane Discord gives a voice channel's chat.
const VOICE_CHAT_WIDTH: f32 = 400.;
const VOICE_CHAT_HEADER_HEIGHT: f32 = 48.;

impl HomeScreen {
    pub(super) fn render_channel_content(&self, cx: &Context<Self>) -> AnyElement {
        let Some(channel) = self.selected_channel_info().cloned() else {
            return pane(message_list::skeleton().into_any_element(), cx);
        };

        let pane = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_channel_header(&channel, cx));

        if channel.kind.is_voice() {
            return pane
                .child(
                    // A plain row rather than `h_flex`, which centres its
                    // children instead of stretching them to its height.
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(self.render_voice_stage(cx))
                        .when(self.voice_chat_open, |this| {
                            this.child(self.render_voice_chat(&channel, cx))
                        }),
                )
                .into_any_element();
        }

        let conversation = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(self.render_messages(cx))
            .child(self.render_message_bar(channel.can_send, cx));

        pane.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(conversation)
                .children(self.render_member_list(cx)),
        )
        .into_any_element()
    }

    fn render_voice_chat(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .w(px(VOICE_CHAT_WIDTH))
            .flex_shrink_0()
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .h(px(VOICE_CHAT_HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Icon::default()
                            .path("icons/message-circle.svg")
                            .size_5()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(channel.name.clone()),
                    )
                    .child(
                        Button::new("close-voice-chat")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .tooltip("Close Chat")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.close_voice_chat(window, cx)
                                }),
                            ),
                    ),
            )
            .child(self.render_messages(cx))
            .child(self.render_message_bar(channel.can_send, cx))
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
                .when_some(channel.topic.as_ref(), |this, topic| {
                    this.child(Separator::vertical().h(px(24.))).child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme::secondary_text(cx))
                            .child(self.render_inline_markdown(
                                topic,
                                MarkdownOptions::new(format!("topic-{}", channel.id), &[]),
                                cx,
                            )),
                    )
                })
                .when(channel.topic.is_none(), |this| this.child(div().flex_1()))
                .when(channel.kind.is_voice(), |this| {
                    let open = self.voice_chat_open;
                    this.child(
                        header_actions().child(
                            Button::new("toggle-voice-chat")
                                .icon(Icon::default().path("icons/message-circle.svg"))
                                .ghost()
                                .small()
                                .selected(open)
                                .tooltip(if open { "Hide Chat" } else { "Show Chat" })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    match this.selected_channel {
                                        Some(_) if open => this.close_voice_chat(window, cx),
                                        Some(id) => this.open_voice_chat(id, window, cx),
                                        None => {}
                                    }
                                })),
                        ),
                    )
                })
                .when(!channel.kind.is_voice(), |this| {
                    let hidden = self.member_list_hidden;
                    this.child(
                        header_actions()
                            .child(self.render_pins_button(cx))
                            .child(
                                Button::new("toggle-member-list")
                                    .icon(Icon::default().path("icons/users.svg"))
                                    .ghost()
                                    .small()
                                    .selected(!hidden)
                                    .tooltip(if hidden {
                                        "Show Member List"
                                    } else {
                                        "Hide Member List"
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_member_list(cx)),
                                    ),
                            )
                            .child(self.render_search_box(cx)),
                    )
                }),
            cx,
        )
    }
}
