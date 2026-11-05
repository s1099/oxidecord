//! The guild-channel conversation: its header, messages, and composer.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, separator::Separator,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::discord::{Channel, ChannelKind};
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
            .children(self.render_thread_notice(&channel, cx))
            .child(self.render_message_bar(channel.can_send, cx));

        pane.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(conversation)
                .map(|this| {
                    if let Some(draft) = self.render_thread_draft(cx) {
                        return this.child(draft.into_any_element());
                    }
                    match self.render_search_panel(cx) {
                        Some(panel) => this.child(panel.into_any_element()),
                        None => this.children(
                            self.render_member_list(cx)
                                .map(IntoElement::into_any_element),
                        ),
                    }
                }),
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

    /// Above a thread's composer, as Discord words it: sending reopens an archived
    /// thread, and a locked one only takes moderators.
    fn render_thread_notice(
        &self,
        channel: &Channel,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let thread = self.selected_thread()?;
        let (icon, text) = if thread.locked {
            (
                "icons/lock.svg",
                "This thread is locked. Only moderators can send messages or reopen it.",
            )
        } else if thread.archived && channel.can_send {
            (
                "icons/archive.svg",
                "This thread is archived. Sending a message will unarchive it.",
            )
        } else {
            return None;
        };
        let theme = cx.theme();
        Some(
            h_flex()
                .flex_shrink_0()
                .mx_2()
                .mb_1()
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .rounded(crate::ui::depth::radius::ITEM)
                .bg(theme.muted.opacity(0.5))
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Icon::default().path(icon).small())
                .child(div().flex_1().min_w_0().child(text)),
        )
    }

    /// `#parent > thread`, as Discord heads a thread opened in full; the parent's
    /// name goes back to it.
    fn render_thread_title(&self, channel: &Channel, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let parent = channel
            .parent_id
            .and_then(|id| self.channel_info(id))
            .map(|parent| (parent.id, parent.name.clone(), parent.kind));
        let thread = self.selected_thread();
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .when_some(parent, |this, (parent_id, name, kind)| {
                this.child(
                    h_flex()
                        .id("thread-parent")
                        .flex_shrink_0()
                        .gap_1()
                        .items_center()
                        .cursor_pointer()
                        .text_color(theme.muted_foreground)
                        .hover(|this| this.text_color(theme.foreground))
                        .child(Icon::default().path(channel_icon_path(kind)).size_5())
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_channel(parent_id, window, cx);
                        })),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .small()
                        .text_color(theme.muted_foreground),
                )
            })
            .child(
                Icon::default()
                    .path(if thread.is_some_and(|thread| thread.private) {
                        "icons/lock.svg"
                    } else {
                        "icons/threads.svg"
                    })
                    .size_5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(channel.name.clone()),
            )
            .when(thread.is_some_and(|thread| thread.archived), |this| {
                this.child(status_tag("Archived", cx))
            })
            .when(thread.is_some_and(|thread| thread.locked), |this| {
                this.child(status_tag("Locked", cx))
            })
    }

    /// Joining, and a menu for leaving, closing and reopening.
    fn render_thread_actions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let thread = self.selected_thread()?;
        let channel = self.selected_channel_info()?;
        let thread_id = thread.id;
        let joined = thread.membership.is_joined();
        let mine = thread.owner_id.is_some() && thread.owner_id == self.self_user_id;
        let can_close = !thread.archived && (mine || channel.can_manage_threads);
        let can_reopen = thread.archived
            && if thread.locked {
                channel.can_manage_threads
            } else {
                channel.can_send_in_threads
            };
        // Archived threads can be neither joined nor left.
        let can_join = !joined && !thread.archived;
        let can_leave = joined && !thread.archived;
        let screen = cx.entity().downgrade();

        let more = (can_leave || can_close || can_reopen).then(|| {
            Button::new("thread-more")
                .icon(IconName::Ellipsis)
                .ghost()
                .small()
                .tooltip("More")
                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                    let item = |label: &'static str, icon: &'static str, run: ThreadAction| {
                        let screen = screen.clone();
                        PopupMenuItem::new(label)
                            .icon(Icon::default().path(icon))
                            .on_click(move |_, window, cx| {
                                if let Some(screen) = screen.upgrade() {
                                    screen.update(cx, |this, cx| run(this, thread_id, window, cx));
                                }
                            })
                    };
                    menu.when(can_close, |menu| {
                        menu.item(item(
                            "Close Thread",
                            "icons/archive.svg",
                            |this, id, window, cx| this.set_thread_archived(id, true, window, cx),
                        ))
                    })
                    .when(can_reopen, |menu| {
                        menu.item(item(
                            "Reopen Thread",
                            "icons/archive-restore.svg",
                            |this, id, window, cx| this.set_thread_archived(id, false, window, cx),
                        ))
                    })
                    .when(can_leave, |menu| {
                        menu.item(item(
                            "Leave Thread",
                            "icons/log-out.svg",
                            |this, id, _, cx| this.set_thread_joined(id, false, cx),
                        ))
                    })
                })
        });

        Some(
            h_flex()
                .gap_2()
                .items_center()
                .when(can_join, |this| {
                    this.child(
                        Button::new("join-thread")
                            .icon(Icon::default().path("icons/threads.svg"))
                            .label("Join Thread")
                            .outline()
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_thread_joined(thread_id, true, cx);
                            })),
                    )
                })
                .children(more)
                .into_any_element(),
        )
    }

    fn render_channel_header(&self, channel: &Channel, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if channel.kind == ChannelKind::Thread {
            return header(
                header_content()
                    .child(self.render_thread_title(channel, cx))
                    .child(
                        header_actions()
                            .children(self.render_thread_actions(cx))
                            .child(self.render_threads_button(cx))
                            .child(self.render_pins_button(cx))
                            .child(self.render_search_box(cx)),
                    ),
                cx,
            )
            .into_any_element();
        }

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
                            .when(channel.kind.has_threads(), |this| {
                                this.child(self.render_threads_button(cx))
                            })
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
        .into_any_element()
    }
}

type ThreadAction = fn(&mut HomeScreen, Id<ChannelMarker>, &mut Window, &mut Context<HomeScreen>);

/// A small label beside a thread's name: archived, locked.
fn status_tag(text: &'static str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex_shrink_0()
        .px(px(6.))
        .rounded(px(4.))
        .bg(theme.muted)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(text)
}
