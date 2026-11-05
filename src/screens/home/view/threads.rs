//! Threads in the conversation pane: the header's threads popout, the "New
//! Thread" panel, and the card under a message that started a thread.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Icon, IconName, Sizable as _,
    checkbox::Checkbox, h_flex, input::Input, spinner::Spinner, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::discord::{self, Thread};
use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::ui::button::Button;
use crate::ui::depth::{self, Level, radius};

use super::markdown::MarkdownOptions;
use super::message::CARD_LIST_WIDTH;

const POPOUT_WIDTH: f32 = 480.;
const POPOUT_LIST_HEIGHT: f32 = 440.;
/// Scrolled this close to the end, the next page of older threads is asked for.
const LOAD_MORE_MARGIN: f32 = 120.;
const PANEL_HEADER_HEIGHT: f32 = 48.;
const PREVIEW_AVATAR_SIZE: f32 = 16.;
const SUMMARY_MAX_WIDTH: f32 = 480.;

/// `5m ago`, `3d ago`, as the threads list labels activity.
fn ago(unix: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    match (now - unix).max(0) {
        0..60 => "just now".into(),
        seconds @ 60..3_600 => format!("{}m ago", seconds / 60),
        seconds @ 3_600..86_400 => format!("{}h ago", seconds / 3_600),
        seconds @ 86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
        seconds @ 2_592_000..31_536_000 => format!("{}mo ago", seconds / 2_592_000),
        seconds => format!("{}y ago", seconds / 31_536_000),
    }
}

fn message_count(count: u32) -> String {
    match count {
        1 => "1 Message".into(),
        count => format!("{count} Messages"),
    }
}

impl HomeScreen {
    /// Over the whole app like the pins popout; a layer closes it on an outside click.
    pub(in crate::screens::home) fn render_threads_popup(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let popup = self.threads_popup.as_ref()?;
        let theme = cx.theme();
        let parent = self.channel_info(popup.channel_id);
        let can_create = parent.is_some_and(|parent| {
            parent.can_create_public_threads || parent.can_create_private_threads
        });
        let parent_id = popup.channel_id;

        let header = h_flex()
            .h(px(PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.4))
            .child(Icon::default().path("icons/threads.svg").small())
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Threads"),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&popup.search)
                        .small()
                        .focus_ring(false)
                        .prefix(Icon::default().path("icons/search.svg").small()),
                ),
            )
            .when(can_create, |this| {
                this.child(
                    Button::new("create-thread")
                        .label("Create")
                        .primary()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_thread_draft(parent_id, None, window, cx);
                        })),
                )
            })
            .child(
                Button::new("close-threads")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip("Close")
                    .on_click(cx.listener(|this, _, _, cx| this.close_threads_popup(cx))),
            );

        let mut sections: Vec<AnyElement> = Vec::new();
        let section = |title: String, threads: Vec<&Thread>, cx: &Context<Self>| {
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(title.to_uppercase()),
                )
                .children(
                    threads
                        .into_iter()
                        .map(|thread| self.render_thread_row(thread, cx)),
                )
                .into_any_element()
        };
        if popup.query.is_empty() {
            let (joined, others): (Vec<&Thread>, Vec<&Thread>) = self
                .threads
                .active_in(parent_id)
                .into_iter()
                .partition(|thread| thread.membership.is_joined());
            let archived: Vec<&Thread> = popup
                .archived
                .iter()
                .filter_map(|&id| self.threads.thread(id))
                .filter(|thread| thread.archived)
                .collect();
            let empty = joined.is_empty() && others.is_empty() && archived.is_empty();
            if !joined.is_empty() {
                sections.push(section(format!("{} Joined", joined.len()), joined, cx));
            }
            if !others.is_empty() {
                sections.push(section(
                    format!("{} Other Active", others.len()),
                    others,
                    cx,
                ));
            }
            if !archived.is_empty() {
                sections.push(section("Older Threads".into(), archived, cx));
            }
            if empty && !popup.loading && popup.error.is_none() {
                sections.push(self.render_no_threads(parent_id, can_create, cx));
            }
        } else {
            let results: Vec<&Thread> = popup
                .results
                .iter()
                .filter_map(|&id| self.threads.thread(id))
                .collect();
            if results.is_empty() && !popup.loading && popup.error.is_none() {
                sections.push(
                    div()
                        .p_6()
                        .text_sm()
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child("No threads match that name.")
                        .into_any_element(),
                );
            } else if !results.is_empty() {
                sections.push(section("Search Results".into(), results, cx));
            }
        }
        if popup.loading {
            sections.push(
                h_flex()
                    .justify_center()
                    .py_4()
                    .child(Spinner::new().color(theme.muted_foreground))
                    .into_any_element(),
            );
        }
        if let Some(error) = &popup.error {
            sections.push(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone())
                    .into_any_element(),
            );
        }

        let scroll = popup.scroll.clone();
        let body = v_flex()
            .id("threads-list")
            .max_h(px(POPOUT_LIST_HEIGHT))
            .overflow_y_scroll()
            .track_scroll(&popup.scroll)
            .px_1()
            .pb_2()
            .on_scroll_wheel(cx.listener(move |this, _, _, cx| {
                let max = scroll.max_offset().y;
                if -scroll.offset().y + px(LOAD_MORE_MARGIN) >= max {
                    this.load_threads_page(true, cx);
                }
            }))
            .children(sections);

        let card = v_flex()
            .occlude()
            .w(px(POPOUT_WIDTH))
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body);

        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            // Consumed, so a click on the threads button closes
                            // the popout instead of closing and reopening it.
                            cx.stop_propagation();
                            this.close_threads_popup(cx);
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

    fn render_no_threads(
        &self,
        parent_id: Id<ChannelMarker>,
        can_create: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .items_center()
            .gap_2()
            .py_8()
            .px_6()
            .text_center()
            .child(
                Icon::default()
                    .path("icons/threads.svg")
                    .size_10()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("There are no threads."),
            )
            .child(
                div().text_sm().text_color(theme.muted_foreground).child(
                    "Stay focused on a conversation with a thread — a temporary text channel.",
                ),
            )
            .when(can_create, |this| {
                this.child(
                    Button::new("create-first-thread")
                        .label("Create Thread")
                        .primary()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_thread_draft(parent_id, None, window, cx);
                        })),
                )
            })
            .into_any_element()
    }

    /// The name, then the newest message when it's known, or the count and age.
    fn render_thread_row(&self, thread: &Thread, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let thread_id = thread.id;
        let selected = self.selected_channel == Some(thread_id);
        let latest = self.threads.latest.get(&thread_id);

        let detail = match latest {
            Some(latest) => {
                let name = self
                    .author_name(latest.author_id, &latest.author_name)
                    .to_owned();
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .child(avatar(
                        name.clone(),
                        latest.author_avatar_url.clone(),
                        px(PREVIEW_AVATAR_SIZE),
                    ))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when_some(self.role_color(latest.author_id), |this, color| {
                                this.text_color(rgb(color))
                            })
                            .child(name),
                    )
                    .child(if latest.content.is_empty() {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .italic()
                            .child("Click to see attachment")
                    } else {
                        div().flex_1().min_w_0().truncate().child(
                            self.render_inline_markdown(
                                &latest.preview,
                                MarkdownOptions::new(
                                    format!("thread-preview-{thread_id}"),
                                    &latest.mentions,
                                )
                                .interactive(false),
                                cx,
                            ),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(format!("· {}", ago(thread.last_active()))),
                    )
                    .into_any_element()
            }
            None => div()
                .child(format!(
                    "{} · {}",
                    message_count(thread.message_count),
                    ago(thread.last_active())
                ))
                .into_any_element(),
        };

        v_flex()
            .id(("thread-row", thread_id.get()))
            .px_2()
            .py(px(6.))
            .gap(px(2.))
            .rounded(radius::ITEM)
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.accent))
            .hover(|this| this.bg(theme.accent))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .min_w_0()
                    .when(thread.private, |this| {
                        this.child(
                            Icon::default()
                                .path("icons/lock.svg")
                                .size_3()
                                .text_color(theme.muted_foreground),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(thread.name.clone()),
                    )
                    .when(thread.locked, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Locked"),
                        )
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(detail),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_thread(thread_id, window, cx);
            }))
            .into_any_element()
    }

    /// The "New Thread" panel, where the member list goes.
    pub(in crate::screens::home) fn render_thread_draft(
        &self,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let draft = self.thread_draft.as_ref()?;
        // Left behind in another channel, it isn't shown.
        let parent = self
            .selected_channel_info()
            .filter(|channel| channel.id == draft.parent_id)?;
        let theme = cx.theme();
        let can_choose_private = draft.starter.is_none()
            && parent.can_create_private_threads
            && parent.can_create_public_threads;
        let private =
            draft.private || (draft.starter.is_none() && !parent.can_create_public_threads);

        let label = |text: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text.to_uppercase())
        };

        let header = h_flex()
            .h(px(PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path("icons/threads.svg")
                    .size_5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New Thread"),
            )
            .child(
                Button::new("close-thread-draft")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip("Close")
                    .on_click(cx.listener(|this, _, _, cx| this.close_thread_draft(cx))),
            );

        let body = v_flex()
            .id("thread-draft")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .gap_4()
            .when_some(draft.starter.as_ref(), |this, starter| {
                this.child(self.render_message_card(starter, "thread-draft", cx))
            })
            .child(
                v_flex()
                    .gap_2()
                    .child(label("Thread Name"))
                    .child(Input::new(&draft.name).disabled(draft.creating)),
            )
            .when(draft.starter.is_none(), |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Checkbox::new("private-thread")
                                .label("Private Thread")
                                .checked(private)
                                .disabled(!can_choose_private || draft.creating)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(draft) = &mut this.thread_draft {
                                        draft.private = *checked;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            div()
                                .pl(px(24.))
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(
                                    "Only people you invite and moderators can see this thread.",
                                ),
                        ),
                )
            })
            .when_some(draft.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            });

        let footer = v_flex()
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .child(Input::new(&draft.message).disabled(draft.creating))
            .child(
                h_flex().justify_end().child(
                    Button::new("submit-thread-draft")
                        .label("Create Thread")
                        .primary()
                        .small()
                        .loading(draft.creating)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.submit_thread_draft(window, cx);
                        })),
                ),
            );

        Some(
            v_flex()
                .w(px(CARD_LIST_WIDTH))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.sidebar)
                .child(header)
                .child(body)
                .child(footer),
        )
    }

    /// The thread a message started, if any: what the message carried, or newer
    /// from the gateway. A thread off a message takes the message's id.
    pub(in crate::screens::home::view) fn started_thread<'a>(
        &'a self,
        message: &'a discord::Message,
    ) -> Option<&'a Thread> {
        let id = message
            .thread
            .as_ref()
            .map_or(message.id.cast(), |thread| thread.id);
        self.threads
            .thread(id)
            .filter(|thread| thread.parent_id == self.selected_channel)
            .or(message.thread.as_ref())
    }

    /// Under a message that started a thread: its name, how many messages, and
    /// the newest one. Clicking it opens the thread.
    pub(in crate::screens::home::view) fn render_thread_summary(
        &self,
        thread: &Thread,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let thread_id = thread.id;
        let latest = self.threads.latest.get(&thread_id);

        let preview = match latest {
            Some(latest) => {
                let name = self
                    .author_name(latest.author_id, &latest.author_name)
                    .to_owned();
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .text_sm()
                    .child(avatar(
                        name.clone(),
                        latest.author_avatar_url.clone(),
                        px(PREVIEW_AVATAR_SIZE),
                    ))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when_some(self.role_color(latest.author_id), |this, color| {
                                this.text_color(rgb(color))
                            })
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .map(|this| {
                                if latest.content.is_empty() {
                                    this.italic().child("Click to see attachment")
                                } else {
                                    this.child(
                                        self.render_inline_markdown(
                                            &latest.preview,
                                            MarkdownOptions::new(
                                                format!("thread-summary-{thread_id}"),
                                                &latest.mentions,
                                            )
                                            .interactive(false),
                                            cx,
                                        ),
                                    )
                                }
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(ago(thread.last_active())),
                    )
                    .into_any_element()
            }
            None if thread.message_count == 0 => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("There are no recent messages in this thread.")
                .into_any_element(),
            None => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("Last active {}", ago(thread.last_active())))
                .into_any_element(),
        };

        v_flex()
            .id(("thread-summary", thread_id.get()))
            .max_w(px(SUMMARY_MAX_WIDTH))
            .w_full()
            .p_2()
            .gap_1()
            .rounded(radius::ITEM)
            .bg(theme.muted.opacity(0.5))
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .hover(|this| this.bg(theme.muted))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(thread.name.clone()),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(px(2.))
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.link)
                            .child(message_count(thread.message_count))
                            .child(Icon::new(IconName::ChevronRight).size_3()),
                    ),
            )
            .child(preview)
            .on_click(cx.listener(move |this, _, window, cx| {
                // A drag that selected text and let go here wasn't a click.
                if !this.has_text_selection() {
                    this.open_thread(thread_id, window, cx);
                }
            }))
    }
}
