//! Reacting to a message, applied locally first and rolled back if the request
//! fails, and taking reactions made anywhere else.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Toggles the current user's reaction on a message: clicking a pill they
    /// already reacted with removes it, otherwise it adds theirs.
    pub(in crate::screens::home) fn toggle_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: discord::ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
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

    /// Applies one reaction of the current user to the local tally.
    fn apply_reaction(
        &mut self,
        message_id: Id<MessageMarker>,
        emoji: &discord::ReactionEmoji,
        add: bool,
    ) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            tally_reaction(&mut message.reactions, emoji, add, true);
        }
    }

    /// Takes a change to the reactions on a message in the open conversation,
    /// made by anyone — the current user's own come back this way too.
    pub(in crate::screens::home) fn handle_reaction(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        change: discord::ReactionChange,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(channel_id) || self.messages_loading {
            return;
        }
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        else {
            return;
        };
        let reactions = &mut self.messages[ix].reactions;
        match change {
            discord::ReactionChange::Add { user_id, emoji } => {
                let own = Some(user_id) == self.self_user_id;
                tally_reaction(reactions, &emoji, true, own);
            }
            discord::ReactionChange::Remove { user_id, emoji } => {
                let own = Some(user_id) == self.self_user_id;
                tally_reaction(reactions, &emoji, false, own);
            }
            discord::ReactionChange::RemoveEmoji(emoji) => {
                reactions.retain(|reaction| reaction.emoji != emoji)
            }
            discord::ReactionChange::RemoveAll => reactions.clear(),
        }
        // The pills can gain or lose a row, and the list only remeasures rows
        // it draws, so an off-screen message would keep its old height.
        self.messages_list.splice(ix..ix + 1, 1);
        cx.notify();
    }
}

/// Adds or removes one reactor from a message's reaction tally. A tally that
/// drops to zero is dropped entirely, like Discord.
///
/// The current user's (`own`) reactions are applied optimistically and then
/// echoed back by the gateway, so one that already matches is skipped rather
/// than counted twice.
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
        // Applied optimistically, then echoed back by the gateway.
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
