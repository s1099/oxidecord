//! Reacting to a message, applied locally first and rolled back if the request
//! fails.

use gpui::*;
use twilight_model::id::{Id, marker::MessageMarker};

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
        let Some(token) = discord::load_token() else {
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

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::toggle_reaction(
            token,
            channel_id,
            message_id,
            emoji.clone(),
            add,
            move |result| {
                let _ = tx.send(result);
            },
        );

        cx.spawn(async move |this, cx| {
            let Ok(Err(_)) = rx.await else {
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
            apply_own_reaction(&mut message.reactions, emoji, add);
        }
    }
}

/// Adds or removes the current user from a message's reaction tally. A tally
/// that drops to zero is dropped entirely, like Discord.
fn apply_own_reaction(
    reactions: &mut Vec<discord::Reaction>,
    emoji: &discord::ReactionEmoji,
    add: bool,
) {
    let Some(ix) = reactions
        .iter()
        .position(|reaction| &reaction.emoji == emoji)
    else {
        if add {
            reactions.push(discord::Reaction {
                emoji: emoji.clone(),
                count: 1,
                me: true,
            });
        }
        return;
    };

    let reaction = &mut reactions[ix];
    reaction.me = add;
    if add {
        reaction.count += 1;
    } else {
        reaction.count = reaction.count.saturating_sub(1);
        if reaction.count == 0 {
            reactions.remove(ix);
        }
    }
}
