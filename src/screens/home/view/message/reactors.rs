//! Who reacted to a message: the tooltip over a reaction pill, and the
//! "View Reactions" dialog listing everyone by emoji.
//! Both read the screen's reactor cache and re-render as it fills. An entry is dropped when
//! its reaction changes, so each re-requests the list whenever it finds it missing.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, spinner::Spinner, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::view::avatar;
use crate::ui::button::Button;
use crate::ui::depth::{self, Finish, Level, Lit as _, radius};

/// How many reactors the tooltip names before summing up the rest.
const TOOLTIP_NAMES: usize = 3;

/// Roughly how tall the tooltip is, for its lighting.
const TOOLTIP_HEIGHT: f32 = 48.;

const DIALOG_WIDTH: f32 = 480.;
const DIALOG_LIST_HEIGHT: f32 = 360.;
const EMOJI_COLUMN_WIDTH: f32 = 96.;
const REACTOR_AVATAR_SIZE: f32 = 24.;

impl HomeScreen {
    /// Opens on `emoji`'s tab, or the first reaction's.
    pub(in crate::screens::home) fn view_reactions(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: Option<discord::ReactionEmoji>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = emoji.or_else(|| {
            self.messages
                .iter()
                .find(|message| message.id == message_id)?
                .reactions
                .first()
                .map(|reaction| reaction.emoji.clone())
        }) else {
            return;
        };
        self.load_reactors(message_id, selected.clone(), false, cx);

        let screen = cx.entity();
        let content = cx.new(|cx| ReactionsDialog::new(screen, message_id, selected, cx));
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .overlay_closable(true)
                .border_color(depth::ring(cx))
                .w(px(DIALOG_WIDTH))
                .title("Reactions")
                .child(content.clone())
        });
    }
}

/// Asks the screen for who reacted with `emoji`, if it isn't known or coming.
fn ensure_reactors(
    screen: &Entity<HomeScreen>,
    message_id: Id<MessageMarker>,
    emoji: &discord::ReactionEmoji,
    cx: &mut App,
) {
    let key = (message_id, emoji.clone());
    if screen.read(cx).reactors.contains_key(&key) {
        return;
    }
    screen.update(cx, |this, cx| {
        this.load_reactors(message_id, emoji.clone(), false, cx)
    });
}

fn emoji_element(emoji: &discord::ReactionEmoji, size: Pixels) -> AnyElement {
    match (emoji.image_url(), emoji) {
        (Some(url), _) => img(url).size(size).into_any_element(),
        (None, discord::ReactionEmoji::Unicode(text)) => div()
            .text_size(size * 0.9)
            .line_height(size)
            .child(text.clone())
            .into_any_element(),
        (None, _) => div().size(size).into_any_element(),
    }
}

/// Discord's "A, B and C reacted with :x:" line. `names` are the first few
/// reactors fetched; `count` is everyone, so the rest are summed up.
fn reacted_line(names: &[&str], count: u64, label: &str) -> String {
    let shown = names.len().min(TOOLTIP_NAMES);
    let others = count.saturating_sub(shown as u64);
    let names = &names[..shown];
    let who = match (names, others) {
        ([], _) => format!("{count} {}", if count == 1 { "person" } else { "people" }),
        ([only], 0) => only.to_string(),
        ([init @ .., last], 0) => format!("{} and {last}", init.join(", ")),
        (names, others) => format!(
            "{} and {others} {}",
            names.join(", "),
            if others == 1 { "other" } else { "others" }
        ),
    };
    format!("{who} reacted with {label}")
}

pub(super) struct ReactionTooltip {
    screen: Entity<HomeScreen>,
    message_id: Id<MessageMarker>,
    emoji: discord::ReactionEmoji,
    _screen_changed: Subscription,
}

impl ReactionTooltip {
    pub(super) fn new(
        screen: Entity<HomeScreen>,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        cx: &mut Context<Self>,
    ) -> Self {
        let _screen_changed = cx.observe(&screen, |this: &mut Self, screen, cx| {
            ensure_reactors(&screen, this.message_id, &this.emoji, cx);
            cx.notify();
        });
        Self {
            screen,
            message_id,
            emoji,
            _screen_changed,
        }
    }
}

impl Render for ReactionTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.screen.read(cx);
        let count = screen
            .messages
            .iter()
            .find(|message| message.id == self.message_id)
            .and_then(|message| {
                message
                    .reactions
                    .iter()
                    .find(|reaction| reaction.emoji == self.emoji)
            })
            .map_or(0, |reaction| reaction.count);
        let names: Vec<&str> = screen
            .reactors
            .get(&(self.message_id, self.emoji.clone()))
            .map(|reactors| {
                reactors
                    .users
                    .iter()
                    .take(TOOLTIP_NAMES)
                    .map(|user| user.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let line = reacted_line(&names, count, &self.emoji.label());
        let theme = cx.theme();

        // Wrapped, so the margin that keeps it clear of the cursor isn't part
        // of the lit box — as `ui::tooltip` does.
        div().child(
            h_flex()
                .m_3()
                .max_w(px(280.))
                .gap_2()
                .bg(theme.foreground)
                .lit(
                    Level::RaisedStrong,
                    Finish::Matte,
                    px(TOOLTIP_HEIGHT),
                    radius::ITEM,
                    cx,
                )
                .px_3()
                .py_2()
                .font_family(theme.font_family.clone())
                .text_xs()
                .text_color(theme.background)
                .child(
                    div()
                        .flex_shrink_0()
                        .child(emoji_element(&self.emoji, px(32.))),
                )
                .child(div().min_w_0().child(line)),
        )
    }
}

struct ReactionsDialog {
    screen: Entity<HomeScreen>,
    message_id: Id<MessageMarker>,
    selected: discord::ReactionEmoji,
    _screen_changed: Subscription,
}

impl ReactionsDialog {
    fn new(
        screen: Entity<HomeScreen>,
        message_id: Id<MessageMarker>,
        selected: discord::ReactionEmoji,
        cx: &mut Context<Self>,
    ) -> Self {
        let _screen_changed = cx.observe(&screen, |this: &mut Self, screen, cx| {
            ensure_reactors(&screen, this.message_id, &this.selected, cx);
            cx.notify();
        });
        Self {
            screen,
            message_id,
            selected,
            _screen_changed,
        }
    }

    fn select(&mut self, emoji: discord::ReactionEmoji, cx: &mut Context<Self>) {
        ensure_reactors(&self.screen, self.message_id, &emoji, cx);
        self.selected = emoji;
        cx.notify();
    }
}

impl Render for ReactionsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let screen = self.screen.read(cx);
        let message_id = self.message_id;
        let reactions = screen
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .map(|message| message.reactions.clone())
            .unwrap_or_default();
        if reactions.is_empty() {
            return div()
                .h(px(DIALOG_LIST_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("No reactions")
                .into_any_element();
        }
        let reactors = screen.reactors.get(&(message_id, self.selected.clone()));

        let emoji_column = v_flex()
            .id("reactions-dialog-emoji")
            .flex_shrink_0()
            .w(px(EMOJI_COLUMN_WIDTH))
            .h_full()
            .gap_0p5()
            .pr_2()
            .border_r_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .children(reactions.iter().enumerate().map(|(ix, reaction)| {
                let selected = reaction.emoji == self.selected;
                let emoji = reaction.emoji.clone();
                h_flex()
                    .id(("reactions-dialog-emoji", ix))
                    .flex_shrink_0()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(radius::ITEM)
                    .cursor_pointer()
                    .when(selected, |this| this.bg(theme.secondary))
                    .when(!selected, |this| {
                        this.hover(|this| this.bg(theme.secondary_hover.opacity(0.5)))
                    })
                    .child(emoji_element(&reaction.emoji, px(20.)))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if selected {
                                theme.foreground
                            } else {
                                theme.muted_foreground
                            })
                            .child(reaction.count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(emoji.clone(), cx)))
            }));

        let users = reactors
            .map(|reactors| reactors.users.as_slice())
            .unwrap_or_default();
        let loading = reactors.is_none_or(|reactors| reactors.loading);
        let error = reactors.and_then(|reactors| reactors.error.clone());
        let more = reactors.is_some_and(|reactors| {
            !reactors.complete && !reactors.loading && reactors.error.is_none()
        });

        let user_column = v_flex()
            .id(("reactions-dialog-users", message_id.get()))
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_0p5()
            .overflow_y_scroll()
            .children(users.iter().map(|user| {
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        avatar(
                            user.name.clone(),
                            user.avatar_url.clone(),
                            px(REACTOR_AVATAR_SIZE),
                        )
                        .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(user.name.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(user.username.clone()),
                    )
            }))
            .when(loading, |this| {
                this.child(
                    h_flex()
                        .justify_center()
                        .py_2()
                        .child(Spinner::new().small().color(theme.muted_foreground)),
                )
            })
            .when_some(error, |this, error| {
                this.child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(error)
                        .child(self.load_more_button("Retry", cx)),
                )
            })
            .when(more, |this| {
                this.child(
                    h_flex()
                        .justify_center()
                        .py_2()
                        .child(self.load_more_button("Load More", cx)),
                )
            });

        h_flex()
            .h(px(DIALOG_LIST_HEIGHT))
            .items_start()
            .gap_2()
            .child(emoji_column)
            .child(user_column)
            .into_any_element()
    }
}

impl ReactionsDialog {
    fn load_more_button(&self, label: &'static str, cx: &Context<Self>) -> Button {
        let screen = self.screen.clone();
        let message_id = self.message_id;
        let emoji = self.selected.clone();
        Button::new(label)
            .small()
            .label(label)
            .on_click(cx.listener(move |_, _, _, cx| {
                screen.update(cx, |this, cx| {
                    this.load_reactors(message_id, emoji.clone(), true, cx)
                });
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::reacted_line;

    #[test]
    fn names_the_first_three_and_sums_the_rest() {
        assert_eq!(reacted_line(&["A"], 1, ":x:"), "A reacted with :x:");
        assert_eq!(
            reacted_line(&["A", "B"], 2, ":x:"),
            "A and B reacted with :x:"
        );
        assert_eq!(
            reacted_line(&["A", "B", "C"], 3, ":x:"),
            "A, B and C reacted with :x:"
        );
        assert_eq!(
            reacted_line(&["A", "B", "C", "D"], 4, ":x:"),
            "A, B, C and 1 other reacted with :x:"
        );
        assert_eq!(
            reacted_line(&["A", "B", "C"], 9, ":x:"),
            "A, B, C and 6 others reacted with :x:"
        );
        assert_eq!(reacted_line(&[], 2, ":x:"), "2 people reacted with :x:");
    }
}
