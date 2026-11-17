//! A poll attached to a message, its tally, and the one the user is composing.

use twilight_model::poll::PollMedia;

use super::message::ReactionEmoji;

#[derive(Clone)]
pub struct Poll {
    pub question: String,
    pub answers: Vec<PollAnswer>,
    pub allow_multiselect: bool,
    /// Unix seconds. `None` for a poll that never expires.
    pub expiry: Option<i64>,
    /// Discord has counted the votes for good; until then counts can lag.
    pub finalized: bool,
}

#[derive(Clone)]
pub struct PollAnswer {
    pub id: u8,
    pub text: String,
    pub emoji: Option<ReactionEmoji>,
    pub count: u64,
    pub me_voted: bool,
}

impl Poll {
    pub fn total_votes(&self) -> u64 {
        self.answers.iter().map(|answer| answer.count).sum()
    }

    pub fn has_voted(&self) -> bool {
        self.answers.iter().any(|answer| answer.me_voted)
    }

    pub fn is_closed(&self, now: i64) -> bool {
        self.finalized || self.expiry.is_some_and(|expiry| expiry <= now)
    }

    /// Takes a later copy's tally and state. `me_voted` stays as it was: a
    /// gateway update is broadcast, so its own `me_voted` says nothing about the user.
    pub fn apply_update(&mut self, updated: Poll) {
        let mine: Vec<u8> = self
            .answers
            .iter()
            .filter(|answer| answer.me_voted)
            .map(|answer| answer.id)
            .collect();
        *self = updated;
        for answer in &mut self.answers {
            answer.me_voted = mine.contains(&answer.id);
        }
    }

    /// Counts one vote added or taken back. The user's own vote is skipped when
    /// it's already been counted, as it is when the vote was cast here.
    pub fn apply_vote(&mut self, answer_id: u8, add: bool, own: bool) {
        let Some(answer) = self
            .answers
            .iter_mut()
            .find(|answer| answer.id == answer_id)
        else {
            return;
        };
        if own {
            if answer.me_voted == add {
                return;
            }
            answer.me_voted = add;
        }
        answer.count = if add {
            answer.count + 1
        } else {
            answer.count.saturating_sub(1)
        };
    }

    /// The user's vote replaced by `picks`, counted at once as Discord's client does.
    pub fn set_own_votes(&mut self, picks: &[u8]) {
        for answer in &mut self.answers {
            let picked = picks.contains(&answer.id);
            if answer.me_voted != picked {
                answer.me_voted = picked;
                answer.count = if picked {
                    answer.count + 1
                } else {
                    answer.count.saturating_sub(1)
                };
            }
        }
    }

    /// Unix seconds as of now, which is what `expiry` is compared against.
    pub fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |now| now.as_secs() as i64)
    }

    /// The footer's "3 votes · 5h left".
    pub fn status_label(&self, now: i64) -> String {
        let total = self.total_votes();
        let votes = if total == 1 {
            "1 vote".to_owned()
        } else {
            format!("{total} votes")
        };
        if self.is_closed(now) {
            return format!("{votes} · Poll closed");
        }
        match self.expiry {
            Some(expiry) => format!("{votes} · {} left", time_left(expiry - now)),
            None => votes,
        }
    }

    /// What a pending message shows until Discord's copy arrives.
    pub fn from_draft(draft: &PollDraft, now: i64) -> Self {
        Self {
            question: draft.question.clone(),
            answers: draft
                .answers
                .iter()
                .enumerate()
                .map(|(ix, text)| PollAnswer {
                    id: ix as u8 + 1,
                    text: text.clone(),
                    emoji: None,
                    count: 0,
                    me_voted: false,
                })
                .collect(),
            allow_multiselect: draft.allow_multiselect,
            expiry: Some(now + i64::from(draft.duration_hours) * 3600),
            finalized: false,
        }
    }
}

/// The largest unit that fits, as the client words it.
fn time_left(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (amount, unit) = match seconds {
        ..60 => (seconds, "s"),
        60..3600 => (seconds / 60, "m"),
        3600..86_400 => (seconds / 3600, "h"),
        _ => (seconds / 86_400, "d"),
    };
    format!("{amount}{unit}")
}

/// A poll as the user composes it, before it's a message.
#[derive(Clone)]
pub struct PollDraft {
    pub question: String,
    pub answers: Vec<String>,
    pub duration_hours: u32,
    pub allow_multiselect: bool,
}

impl PollDraft {
    pub const MAX_QUESTION: usize = 300;
    pub const MAX_ANSWER: usize = 55;
    pub const MAX_ANSWERS: usize = 10;

    /// Create Message's `poll`, which takes a duration where a sent poll has an expiry.
    pub(in crate::discord) fn to_json(&self) -> serde_json::Value {
        let answers: Vec<_> = self
            .answers
            .iter()
            .map(|text| serde_json::json!({ "poll_media": { "text": text } }))
            .collect();
        serde_json::json!({
            "question": { "text": self.question },
            "answers": answers,
            "duration": self.duration_hours,
            "allow_multiselect": self.allow_multiselect,
            "layout_type": 1,
        })
    }
}

pub(in crate::discord) fn convert_poll(poll: twilight_model::poll::Poll) -> Poll {
    let results = poll.results;
    let tally = |id: u8| {
        results
            .as_ref()
            .and_then(|results| results.answer_counts.iter().find(|count| count.id == id))
    };
    Poll {
        question: poll.question.text.unwrap_or_default(),
        answers: poll
            .answers
            .into_iter()
            .map(|answer| {
                let count = tally(answer.answer_id);
                PollAnswer {
                    id: answer.answer_id,
                    count: count.map_or(0, |count| count.count),
                    me_voted: count.is_some_and(|count| count.me_voted),
                    emoji: convert_emoji(&answer.poll_media),
                    text: answer.poll_media.text.unwrap_or_default(),
                }
            })
            .collect(),
        allow_multiselect: poll.allow_multiselect,
        expiry: poll.expiry.map(|expiry| expiry.as_secs()),
        finalized: results.as_ref().is_some_and(|results| results.is_finalized),
    }
}

fn convert_emoji(media: &PollMedia) -> Option<ReactionEmoji> {
    let emoji = media.emoji.as_ref()?;
    Some(match emoji.id {
        Some(id) => ReactionEmoji::Custom {
            id,
            name: emoji.name.clone().unwrap_or_default(),
            animated: emoji.animated,
        },
        None => ReactionEmoji::Unicode(emoji.name.clone()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll() -> Poll {
        Poll::from_draft(
            &PollDraft {
                question: "q".into(),
                answers: vec!["a".into(), "b".into()],
                duration_hours: 1,
                allow_multiselect: false,
            },
            0,
        )
    }

    #[test]
    fn own_vote_echo_is_not_counted_twice() {
        let mut poll = poll();
        poll.set_own_votes(&[1]);
        poll.apply_vote(1, true, true);
        assert_eq!(poll.answers[0].count, 1);
        poll.set_own_votes(&[2]);
        assert_eq!((poll.answers[0].count, poll.answers[1].count), (0, 1));
    }

    #[test]
    fn time_left_takes_the_largest_unit() {
        assert_eq!(time_left(30), "30s");
        assert_eq!(time_left(5 * 3600 + 10), "5h");
        assert_eq!(time_left(3 * 86_400), "3d");
    }
}
