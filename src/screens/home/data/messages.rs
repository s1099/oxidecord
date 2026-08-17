//! Opening a conversation, paging its history, sending to it, and keeping it
//! live over the gateway.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

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

impl HomeScreen {
    pub(crate) fn select_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel == Some(channel_id) {
            return;
        }
        self.selected_channel = Some(channel_id);
        self.load_messages(window, cx);
    }

    fn load_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        self.messages.clear();
        self.messages_list.reset(0);
        // Release the previous channel's decoded images instead of letting them
        // accumulate; the new channel repopulates the cache as it renders.
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.messages_error = None;
        self.older_loading = false;
        self.reached_oldest = false;
        // A freshly opened channel starts pinned to its newest message, so
        // live messages should follow along until the user scrolls up.
        self.at_bottom = true;
        self.send_error = None;
        self.replying_to = None;
        self.pending_attachments.clear();
        self.messages_loading = true;

        let placeholder = match self.view {
            View::DirectMessages => self
                .selected_dm_info()
                .map(|dm| format!("Message @{}", dm.name))
                .unwrap_or_else(|| "Send a message".into()),
            View::Guild => self
                .selected_channel_info()
                .map(|channel| format!("Message #{}", channel.name))
                .unwrap_or_else(|| "Send a message".into()),
        };
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.messages_loading = false;
            self.messages_error = Some("No token found. Please log in first.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, None, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // The user may have clicked another channel while this request
                // was in flight; drop the stale response.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    Ok(messages) => {
                        this.reached_oldest = messages.len() < discord::MESSAGE_PAGE_SIZE;
                        this.messages = messages;
                        // Resetting also snaps the bottom-aligned list to the
                        // newest message.
                        this.messages_list.reset(this.messages.len());
                    }
                    Err(err) => this.messages_error = Some(err),
                }
                this.messages_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetches the page of messages older than the oldest loaded one and
    /// prepends it, keeping the current scroll position.
    pub(crate) fn load_older_messages(&mut self, cx: &mut Context<Self>) {
        if self.messages_loading || self.older_loading || self.reached_oldest {
            return;
        }
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(oldest_id) = self.messages.first().map(|message| message.id) else {
            return;
        };
        let Some(token) = discord::load_token() else {
            return;
        };

        self.older_loading = true;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_messages(token, channel_id, Some(oldest_id), move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the channel changed or the messages
                // were reloaded while this request was in flight.
                if this.selected_channel != Some(channel_id)
                    || this.messages.first().map(|message| message.id) != Some(oldest_id)
                {
                    return;
                }
                this.older_loading = false;
                match result {
                    Ok(older) => {
                        if older.len() < discord::MESSAGE_PAGE_SIZE {
                            this.reached_oldest = true;
                        }
                        let count = older.len();
                        if count > 0 {
                            this.messages.splice(0..0, older);
                            this.messages_list.splice(0..0, count);
                        }
                    }
                    // Keep the messages on screen; the fetch retries the next
                    // time the user scrolls near the top.
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let content = self.message_input.read(cx).value().trim().to_string();
        if content.is_empty() && self.pending_attachments.is_empty() {
            return;
        }

        let Some(token) = discord::load_token() else {
            self.send_error = Some("No token found. Please log in first.".into());
            cx.notify();
            return;
        };

        let reply_to = self.replying_to.as_ref().map(|target| target.message_id);
        let attachments: Vec<(String, Vec<u8>)> = self
            .pending_attachments
            .drain(..)
            .map(|attachment| (attachment.filename, attachment.data.bytes().to_vec()))
            .collect();

        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_error = None;
        self.replying_to = None;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::send_message(
            token,
            channel_id,
            content,
            reply_to,
            attachments,
            move |result| {
                let _ = tx.send(result);
            },
        );

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update(cx, |this, cx| {
                // Drop the response if the user switched channels meanwhile.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                match result {
                    // The sent message is rendered when it arrives back over the
                    // gateway as a `MESSAGE_CREATE`, so nothing to append here;
                    // appending it too would duplicate it in the list.
                    Ok(_) => {}
                    Err(err) => this.send_error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Toggles the current user's reaction on a message: clicking a pill they
    /// already reacted with removes it, otherwise it adds theirs. The tally is
    /// updated straight away and rolled back if the request fails.
    pub(crate) fn toggle_reaction(
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

    /// Opens the gateway connection and pumps live `MESSAGE_CREATE` events
    /// onto the gpui foreground, where they update the open conversation.
    pub(crate) fn start_gateway(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::mpsc::unbounded::<discord::IncomingMessage>();
        discord::connect_gateway(token, move |incoming| {
            // Returns whether the foreground receiver is still around; once it
            // isn't (the screen was dropped), the gateway loop stops.
            tx.unbounded_send(incoming).is_ok()
        });

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(incoming) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_incoming_message(incoming, cx))
                    .is_err()
                {
                    // The entity is gone; stop draining so the sender closes.
                    break;
                }
            }
        })
        .detach();
    }

    /// Appends a live message to the open conversation, if it belongs there.
    fn handle_incoming_message(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(incoming.channel_id) {
            return;
        }
        // The history for this channel is still loading and will replace the
        // whole list when it lands (and include this message), so skip it now
        // to avoid a desync between `messages` and the list state.
        if self.messages_loading {
            return;
        }
        // Ignore duplicates: the echo of a message we just sent ourselves, or a
        // repeated dispatch.
        if self
            .messages
            .iter()
            .any(|message| message.id == incoming.message.id)
        {
            return;
        }

        let ix = self.messages.len();
        self.messages.push(incoming.message);
        self.messages_list.splice(ix..ix, 1);
        // Follow the conversation only when the newest message was already in
        // view; if the user has scrolled up to read history, leave them there.
        if self.at_bottom {
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }
}
