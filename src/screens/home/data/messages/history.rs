//! Opening a conversation and paging back through its history.

use gpui::*;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(in crate::screens::home) fn select_channel(
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
    pub(in crate::screens::home) fn load_older_messages(&mut self, cx: &mut Context<Self>) {
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
}
