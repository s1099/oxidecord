//! Voting on polls (counted at once, rolled back on failure), ending one's own,
//! and taking others' votes from the gateway.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// DMs always allow polls.
    pub(in crate::screens::home) fn can_send_polls(&self) -> bool {
        self.view == View::DirectMessages
            || self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_send_polls)
    }

    fn poll_mut(&mut self, message_id: Id<MessageMarker>) -> Option<&mut discord::Poll> {
        self.messages
            .iter_mut()
            .find(|message| message.id == message_id)?
            .poll
            .as_mut()
    }

    /// A single-answer poll swaps the tick; a multi-answer one toggles it.
    pub(in crate::screens::home) fn pick_poll_answer(
        &mut self,
        message_id: Id<MessageMarker>,
        answer_id: u8,
        cx: &mut Context<Self>,
    ) {
        let Some(multi) = self.poll_mut(message_id).map(|poll| poll.allow_multiselect) else {
            return;
        };
        let picks = self.poll_picks.entry(message_id).or_default();
        if let Some(ix) = picks.iter().position(|&id| id == answer_id) {
            picks.remove(ix);
        } else if multi {
            picks.push(answer_id);
        } else {
            *picks = vec![answer_id];
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn vote_poll(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        let picks = self.poll_picks.remove(&message_id).unwrap_or_default();
        if !picks.is_empty() {
            self.set_poll_vote(message_id, picks, cx);
        }
    }

    pub(in crate::screens::home) fn remove_poll_vote(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        self.set_poll_vote(message_id, Vec::new(), cx);
    }

    fn set_poll_vote(
        &mut self,
        message_id: Id<MessageMarker>,
        picks: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(poll) = self.poll_mut(message_id) else {
            return;
        };
        let before: Vec<u8> = poll
            .answers
            .iter()
            .filter(|answer| answer.me_voted)
            .map(|answer| answer.id)
            .collect();
        poll.set_own_votes(&picks);
        self.poll_results_shown.remove(&message_id);
        self.remeasure_message(message_id);
        cx.notify();

        let request = discord::vote_poll(channel_id, message_id, picks);
        cx.spawn(async move |this, cx| {
            let Err(err) = request.await else {
                return;
            };
            log::error!("poll vote failed: {err}");
            let _ = this.update(cx, |this, cx| {
                if let Some(poll) = this.poll_mut(message_id) {
                    poll.set_own_votes(&before);
                    this.remeasure_message(message_id);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Before voting, shows the tally in place of the answers to pick, and back.
    pub(in crate::screens::home) fn toggle_poll_results(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        if !self.poll_results_shown.remove(&message_id) {
            self.poll_results_shown.insert(message_id);
        }
        self.remeasure_message(message_id);
        cx.notify();
    }

    pub(in crate::screens::home) fn end_poll(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let request = discord::end_poll(channel_id, message_id);
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(ended) => {
                    if let Some(message) = this
                        .messages
                        .iter_mut()
                        .find(|message| message.id == ended.id)
                    {
                        message.apply_edit(ended);
                        this.remeasure_message(message_id);
                        cx.notify();
                    }
                }
                Err(err) => {
                    this.send_error = Some(format!("Couldn't end the poll: {err}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Includes the user's own votes, which come back this way too.
    pub(in crate::screens::home) fn handle_poll_vote(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        user_id: Id<UserMarker>,
        answer_id: u8,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let own = Some(user_id) == self.self_user_id;
        if self.selected_channel != Some(channel_id) || self.messages_loading {
            if let Some(poll) = self
                .history_cache
                .message_mut(channel_id, message_id)
                .and_then(|message| message.poll.as_mut())
            {
                poll.apply_vote(answer_id, add, own);
            }
            return;
        }
        if let Some(poll) = self.poll_mut(message_id) {
            poll.apply_vote(answer_id, add, own);
            self.remeasure_message(message_id);
            cx.notify();
        }
    }

    /// The tally can switch the poll between answers to pick and bars, which
    /// differ in height, and the list only remeasures rows it's told about.
    fn remeasure_message(&mut self, message_id: Id<MessageMarker>) {
        if let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        {
            self.splice_messages(ix..ix + 1, 1);
        }
    }
}
