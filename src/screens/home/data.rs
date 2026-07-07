use gpui::*;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker},
    Id,
};

use crate::discord::{self, Channel};

use super::channels::build_channel_groups;
use super::HomeScreen;

impl HomeScreen {
    pub(super) fn load_guilds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            self.loading = false;
            self.error = Some("No token found in auth.json. Please log in first.".into());
            return;
        };

        // `discord::fetch_guilds` runs its callback on a background Tokio
        // thread, but gpui's entity/async handles are `!Send`. Bridge the two
        // with a plain, `Send`-safe channel and let gpui's own (non-Send)
        // foreground task pick up the result.
        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_guilds(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, window, cx);
                        }
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_guild == Some(guild_id) {
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_groups.clear();
        self.selected_channel = None;
        self.collapsed_categories.clear();
        self.channels_error = None;
        self.channels_loading = true;
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.channels_loading = false;
            self.channels_error = Some("No token found in auth.json.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_channels(token, guild_id, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                // The user may have clicked another guild while this request
                // was in flight; drop the stale response.
                if this.selected_guild != Some(guild_id) {
                    return;
                }
                match result {
                    Ok(channels) => {
                        this.channel_groups = build_channel_groups(channels);
                        // Default to the first text-like channel, like Discord.
                        let first = this
                            .channel_groups
                            .iter()
                            .flat_map(|group| &group.channels)
                            .find(|channel| !channel.kind.is_voice())
                            .map(|channel| channel.id);
                        if let Some(channel_id) = first {
                            this.select_channel(channel_id, window, cx);
                        }
                    }
                    Err(err) => this.channels_error = Some(err),
                }
                this.channels_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_channel(
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
        self.send_error = None;
        self.messages_loading = true;

        let placeholder = self
            .selected_channel_info()
            .map(|channel| format!("Message #{}", channel.name))
            .unwrap_or_else(|| "Send a message".into());
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.messages_loading = false;
            self.messages_error = Some("No token found in auth.json.".into());
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
    pub(super) fn load_older_messages(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn send_current_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let content = self.message_input.read(cx).value().trim().to_string();
        if content.is_empty() {
            return;
        }

        let Some(token) = discord::load_token() else {
            self.send_error = Some("No token found in auth.json.".into());
            cx.notify();
            return;
        };

        self.message_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.send_error = None;
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::send_message(token, channel_id, content, move |result| {
            let _ = tx.send(result);
        });

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
                    Ok(message) => {
                        let ix = this.messages.len();
                        this.messages.push(message);
                        this.messages_list.splice(ix..ix, 1);
                        // Past-the-end offsets clamp to the bottom.
                        this.messages_list.scroll_to(ListOffset {
                            item_ix: ix + 1,
                            offset_in_item: px(0.),
                        });
                    }
                    Err(err) => this.send_error = Some(err),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn selected_channel_info(&self) -> Option<&Channel> {
        let id = self.selected_channel?;
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == id)
    }
}
