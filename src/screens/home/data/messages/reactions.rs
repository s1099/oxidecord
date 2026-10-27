//! Reacting to a message (applied locally, rolled back on failure), taking
//! reactions from elsewhere, and fetching who reacted.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        let Some(add) = self
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .and_then(|message| {
                message
                    .reactions
                    .iter()
                    .find(|reaction| reaction.emoji == emoji)
            })
            .map(|reaction| !reaction.me)
        else {
            return;
        };
        self.send_reaction(message_id, emoji, add, cx);
    }

    /// Picking a reaction already made leaves it rather than taking it back.
    pub(in crate::screens::home) fn add_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self
            .messages
            .iter()
            .find(|message| message.id == message_id)
        else {
            return;
        };
        if message
            .reactions
            .iter()
            .any(|reaction| reaction.emoji == emoji && reaction.me)
        {
            return;
        }
        self.send_reaction(message_id, emoji, true, cx);
    }

    fn send_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        self.apply_reaction(message_id, &emoji, add);
        cx.notify();

        let request = discord::toggle_reaction(channel_id, message_id, emoji.clone(), add);
        cx.spawn(async move |this, cx| {
            let Err(_) = request.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Undo the optimistic update. Harmless if the channel changed
                // meanwhile: the message is no longer in the list.
                this.apply_reaction(message_id, &emoji, !add);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: &discord::ReactionEmoji,
        add: bool,
    ) {
        if let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        {
            tally_reaction(&mut self.messages[ix].reactions, emoji, add, true);
            self.forget_reactors(message_id, Some(emoji));
            // A first or last reaction adds or drops the row of pills.
            self.messages_list.splice(ix..ix + 1, 1);
        }
    }

    /// Includes the current user's own, which come back this way too.
    pub(in crate::screens::home) fn handle_reaction(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        change: discord::ReactionChange,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(channel_id) || self.messages_loading {
            // A conversation that was opened before keeps its reactions
            // current for when it's opened again.
            if let Some(message) = self.history_cache.message_mut(channel_id, message_id) {
                apply_reaction(&mut message.reactions, change, self.self_user_id);
            }
            return;
        }
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        else {
            return;
        };
        let changed = apply_reaction(&mut self.messages[ix].reactions, change, self.self_user_id);
        self.forget_reactors(message_id, changed.as_ref());
        // The pills can gain or lose a row, and the list only remeasures rows
        // it draws, so an off-screen message would keep its old height.
        self.messages_list.splice(ix..ix + 1, 1);
        cx.notify();
    }
}

/// Applies a reaction dispatch to a message's reactions, returning the emoji
/// whose reactors changed — `None` when they all did.
fn apply_reaction(
    reactions: &mut Vec<discord::Reaction>,
    change: discord::ReactionChange,
    self_id: Option<Id<UserMarker>>,
) -> Option<discord::ReactionEmoji> {
    match change {
        discord::ReactionChange::Add { user_id, emoji } => {
            tally_reaction(reactions, &emoji, true, Some(user_id) == self_id);
            Some(emoji)
        }
        discord::ReactionChange::Remove { user_id, emoji } => {
            tally_reaction(reactions, &emoji, false, Some(user_id) == self_id);
            Some(emoji)
        }
        discord::ReactionChange::RemoveEmoji(emoji) => {
            reactions.retain(|reaction| reaction.emoji != emoji);
            Some(emoji)
        }
        discord::ReactionChange::RemoveAll => {
            reactions.clear();
            None
        }
    }
}

impl HomeScreen {
    /// Skipped if already known or on its way; `more` fetches the next page of a
    /// cut-short list.
    pub(in crate::screens::home) fn load_reactors(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        more: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let key = (message_id, emoji.clone());
        if !more && self.reactors.contains_key(&key) {
            return;
        }
        let entry = self.reactors.entry(key.clone()).or_default();
        if entry.loading || entry.complete {
            return;
        }
        let after = entry.users.last().map(|user| user.id);
        entry.loading = true;
        entry.error = None;

        let request = discord::fetch_reactors(channel_id, message_id, emoji, after);
        entry._task = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                let Some(entry) = this.reactors.get_mut(&key) else {
                    return;
                };
                entry.loading = false;
                match result {
                    Ok(page) => {
                        entry.complete = page.len() < discord::REACTORS_PAGE_SIZE;
                        entry.users.extend(page);
                    }
                    Err(err) => entry.error = Some(err),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Drops who-reacted for `emoji` (any, on `None`) once it changed. Whatever
    /// still shows the list fetches it again.
    fn forget_reactors(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: Option<&discord::ReactionEmoji>,
    ) {
        self.reactors.retain(|(id, reacted), _| {
            *id != message_id || emoji.is_some_and(|emoji| emoji != reacted)
        });
    }
}

/// A tally dropping to zero is removed entirely, like Discord. The current
/// user's (`own`) reactions apply optimistically and are echoed by the gateway,
/// so one already matching is skipped rather than counted twice.
fn tally_reaction(
    reactions: &mut Vec<discord::Reaction>,
    emoji: &discord::ReactionEmoji,
    add: bool,
    own: bool,
) {
    let Some(ix) = reactions
        .iter()
        .position(|reaction| &reaction.emoji == emoji)
    else {
        if add {
            reactions.push(discord::Reaction {
                emoji: emoji.clone(),
                count: 1,
                me: own,
            });
        }
        return;
    };

    let reaction = &mut reactions[ix];
    if own {
        if reaction.me == add {
            return;
        }
        reaction.me = add;
    }
    if add {
        reaction.count += 1;
    } else {
        reaction.count = reaction.count.saturating_sub(1);
        if reaction.count == 0 {
            reactions.remove(ix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tally_reaction;
    use crate::discord;

    fn thumbs() -> discord::ReactionEmoji {
        discord::ReactionEmoji::Unicode("👍".into())
    }

    #[test]
    fn own_reaction_echo_is_not_counted_twice() {
        let mut reactions = Vec::new();
        tally_reaction(&mut reactions, &thumbs(), true, true);
        tally_reaction(&mut reactions, &thumbs(), true, true);
        assert_eq!(reactions[0].count, 1);
        assert!(reactions[0].me);

        tally_reaction(&mut reactions, &thumbs(), true, false);
        assert_eq!(reactions[0].count, 2);

        tally_reaction(&mut reactions, &thumbs(), false, true);
        tally_reaction(&mut reactions, &thumbs(), false, true);
        assert_eq!(reactions[0].count, 1);
        assert!(!reactions[0].me);

        tally_reaction(&mut reactions, &thumbs(), false, false);
        assert!(reactions.is_empty());
    }
}
