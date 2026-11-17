//! A message's poll: answers to pick and a Vote button until the user votes or
//! it closes, then each answer's share of the votes as a bar.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::ui::button::Button;
use crate::ui::depth::radius;
use crate::ui::twemoji;

const CARD_MAX_WIDTH: f32 = 440.;
const ANSWER_HEIGHT: f32 = 40.;
const INDICATOR_SIZE: f32 = 18.;
const EMOJI_SIZE: f32 = 20.;

impl HomeScreen {
    pub(super) fn render_poll(
        &self,
        message: &discord::Message,
        poll: &discord::Poll,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let message_id = message.id;
        let now = discord::Poll::now();
        let closed = poll.is_closed(now);
        let voted = poll.has_voted();
        // Nothing can be cast on a copy the server doesn't know yet.
        let sent = message.delivery == discord::Delivery::Sent;
        let peeking = self.poll_results_shown.contains(&message_id);
        let tallied = closed || voted || peeking;
        let picks = self
            .poll_picks
            .get(&message_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let total = poll.total_votes();
        // A closed poll marks its leaders, as Discord's client does.
        let top = poll.answers.iter().map(|answer| answer.count).max();

        let answers = poll.answers.iter().map(|answer| {
            let answer_id = answer.id;
            let share = if total == 0 {
                0.
            } else {
                answer.count as f32 / total as f32
            };
            let leading = closed && total > 0 && top == Some(answer.count);
            let picked = picks.contains(&answer_id);
            let highlight = if tallied { answer.me_voted } else { picked };

            h_flex()
                .id(("poll-answer", message_id.get() * 16 + u64::from(answer_id)))
                .relative()
                .w_full()
                .h(px(ANSWER_HEIGHT))
                .px_3()
                .gap_2()
                .items_center()
                .rounded(radius::ITEM)
                .overflow_hidden()
                .border_1()
                .border_color(if highlight {
                    theme.primary
                } else {
                    theme.border
                })
                .bg(theme.secondary.opacity(0.5))
                .when(tallied, |this| {
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .w(relative(share))
                            .bg(if answer.me_voted || leading {
                                theme.primary.opacity(0.25)
                            } else {
                                theme.secondary
                            }),
                    )
                })
                .when(!tallied && sent, |this| {
                    this.cursor_pointer()
                        .hover(|this| this.bg(theme.secondary_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_poll_answer(message_id, answer_id, cx);
                        }))
                })
                .children(answer.emoji.as_ref().map(|emoji| {
                    div()
                        .relative()
                        .flex_shrink_0()
                        .child(self.render_poll_emoji(emoji, cx))
                }))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(answer.text.clone()),
                )
                .map(|this| {
                    if tallied {
                        this.child(
                            h_flex()
                                .relative()
                                .flex_shrink_0()
                                .gap_2()
                                .items_center()
                                .text_sm()
                                .when(answer.me_voted, |this| {
                                    this.child(
                                        Icon::new(IconName::CircleCheck)
                                            .small()
                                            .text_color(theme.primary),
                                    )
                                })
                                .child(
                                    div()
                                        .text_color(theme.muted_foreground)
                                        .child(vote_count(answer.count)),
                                )
                                .child(
                                    div()
                                        .w(px(40.))
                                        .text_right()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("{}%", (share * 100.).round())),
                                ),
                        )
                    } else {
                        this.child(indicator(picked, poll.allow_multiselect, cx))
                    }
                })
        });

        let footer = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(poll.status_label(now)),
            )
            .when(sent && !closed, |this| {
                if voted {
                    this.child(
                        Button::new(("poll-unvote", message_id.get()))
                            .ghost()
                            .small()
                            .label("Remove Vote")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_poll_vote(message_id, cx);
                            })),
                    )
                } else {
                    this.child(
                        Button::new(("poll-peek", message_id.get()))
                            .ghost()
                            .small()
                            .label(if peeking { "Go Back" } else { "Show Results" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_poll_results(message_id, cx);
                            })),
                    )
                    .when(!peeking, |this| {
                        this.child(
                            Button::new(("poll-vote", message_id.get()))
                                .primary()
                                .small()
                                .label("Vote")
                                .disabled(picks.is_empty())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.vote_poll(message_id, cx);
                                })),
                        )
                    })
                }
            });

        v_flex()
            .w_full()
            .max_w(px(CARD_MAX_WIDTH))
            .p_3()
            .gap_2()
            .rounded(radius::CARD)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary.opacity(0.3))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_normal()
                            .child(poll.question.clone()),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        if poll.allow_multiselect {
                            "Select one or more answers"
                        } else {
                            "Select one answer"
                        },
                    )),
            )
            .child(v_flex().gap_1p5().children(answers))
            .child(footer)
    }

    fn render_poll_emoji(&self, emoji: &discord::ReactionEmoji, cx: &App) -> AnyElement {
        match (emoji.image_url(), emoji) {
            (Some(url), _) => img(url)
                .image_cache(&self.image_cache)
                .size(px(EMOJI_SIZE))
                .into_any_element(),
            (None, discord::ReactionEmoji::Unicode(name)) => {
                twemoji::glyph(name, px(EMOJI_SIZE), &twemoji::cache(cx))
            }
            _ => div().into_any_element(),
        }
    }
}

/// A radio for a single-answer poll, a checkbox for a multi-answer one.
fn indicator(picked: bool, multi: bool, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .relative()
        .flex_shrink_0()
        .size(px(INDICATOR_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .map(|this| {
            if multi {
                this.rounded(px(4.))
            } else {
                this.rounded_full()
            }
        })
        .border_2()
        .border_color(if picked {
            theme.primary
        } else {
            theme.muted_foreground
        })
        .when(picked, |this| {
            this.bg(theme.primary).child(
                Icon::new(IconName::Check)
                    .xsmall()
                    .text_color(theme.primary_foreground),
            )
        })
}

fn vote_count(count: u64) -> String {
    if count == 1 {
        "1 vote".into()
    } else {
        format!("{count} votes")
    }
}
