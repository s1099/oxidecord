//! Opening a conversation, at the first unread message when there is one, and
//! paging through its history either way from there.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, MessagePage};
use crate::screens::home::history_cache::merge_latest;
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
        self.save_collapsed_categories();
        self.selected_channel = Some(channel_id);
        if self.view == View::Guild
            && let Some(guild_id) = self.selected_guild
        {
            self.last_guild_channel.insert(guild_id, channel_id);
        }
        // A DM's call that started before this session did is only reported
        // on request, and the call view needs to know it's there.
        if self.view == View::DirectMessages {
            self.last_dm = Some(channel_id);
            if let Some(gateway) = &self.gateway {
                gateway.request_call_connect(channel_id);
            }
        }
        self.load_messages(window, cx);
    }

    pub(in crate::screens::home) fn load_messages(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // Taken whatever opens, so a jump that went astray doesn't wait to
        // fire the next time its channel is opened by hand.
        let jump = self
            .pending_jump
            .take()
            .filter(|&(channel, _)| channel == channel_id)
            .map(|(_, message_id)| message_id);
        let placeholder = match self.view {
            View::DirectMessages => self
                .selected_dm_info()
                .map(|dm| format!("Message @{}", dm.name)),
            // A voice channel's chat goes without the hash, as in Discord.
            View::Guild => self.selected_channel_info().map(|channel| {
                if channel.kind.is_voice() {
                    format!("Message {}", channel.name)
                } else {
                    format!("Message #{}", channel.name)
                }
            }),
        };
        // Where the user had read up to, taken before opening acks it.
        let marker = self
            .unreads
            .is_unread(channel_id)
            .then(|| self.unreads.acked(channel_id))
            .flatten();
        self.reset_conversation(placeholder, window, cx);
        self.unread_marker = marker;
        if let Some(message_id) = jump {
            self.load_around(channel_id, message_id, window, cx);
            return;
        }
        // Opened before, it's drawn from what was there then while the
        // newest page is fetched, which is merged in when it lands.
        match self.history_cache.take(channel_id) {
            Some(cached) => {
                self.messages = cached.messages;
                self.reached_oldest = cached.reached_oldest;
                self.messages_channel = Some(channel_id);
                self.request_author_members();
                self.messages_list.reset(self.messages.len());
            }
            None => self.messages_loading = true,
        }
        self.fetch_opening_page(channel_id, marker, window, cx);
    }

    /// Matches Discord's "Jump To Present".
    pub(in crate::screens::home) fn jump_to_present(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        if self.messages_loading {
            return;
        }
        self.messages.clear();
        self.messages_channel = None;
        self.messages_list.reset(0);
        self.reactors.clear();
        self.older_loading = false;
        self.newer_loading = false;
        self.at_bottom = true;
        self.messages_loading = true;
        self.messages_generation += 1;
        self.fetch_opening_page(channel_id, None, window, cx);
        cx.notify();
    }

    /// The newest page, scrolled to the "new" divider when `unread_after` falls in it.
    /// If it falls further back the unread run exceeds a page, so it opens where the
    /// user left off, as Discord does, to be paged forward.
    fn fetch_opening_page(
        &mut self,
        channel_id: Id<ChannelMarker>,
        unread_after: Option<Id<MessageMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.messages_generation;
        cx.spawn_in(window, async move |this, cx| {
            let page_size = discord::MESSAGE_PAGE_SIZE;
            let mut result = discord::fetch_messages(channel_id, MessagePage::Latest)
                .await
                .map(|page| (page, true));
            if let (Ok((latest, _)), Some(marker)) = (&result, unread_after)
                && latest.len() == page_size
                && latest.first().is_some_and(|first| first.id > marker)
            {
                result = discord::fetch_messages(channel_id, MessagePage::After(marker))
                    .await
                    .map(|page| (page, false));
            }
            let _ = this.update_in(cx, |this, window, cx| {
                // Drop the stale response if the user changed channel or jumped meanwhile.
                if this.selected_channel != Some(channel_id)
                    || this.messages_generation != generation
                {
                    return;
                }
                // Drawn from the cache meanwhile, the newest page is merged
                // into what's shown, which keeps the scroll where it is.
                let refreshing = !this.messages_loading;
                this.messages_loading = false;
                match result {
                    Ok((messages, true)) if refreshing => {
                        let full = messages.len() == page_size;
                        let before = this.messages.len();
                        match merge_latest(&mut this.messages, messages, !full) {
                            Ok(ix) => {
                                if !full {
                                    this.reached_oldest = true;
                                }
                                // An older page asked for of the head that
                                // was just replaced won't land.
                                if ix == 0 {
                                    this.older_loading = false;
                                }
                                this.request_author_members();
                                this.splice_messages(ix..before, this.messages.len() - ix);
                                if this.at_bottom {
                                    this.messages_list.scroll_to(ListOffset {
                                        item_ix: this.messages.len(),
                                        offset_in_item: px(0.),
                                    });
                                }
                                if unread_after.is_some() {
                                    this.scroll_to_unread(window, cx);
                                }
                            }
                            Err(page) => {
                                this.show_opening_page(page, true, unread_after, window, cx)
                            }
                        }
                    }
                    Ok((messages, latest)) => {
                        this.show_opening_page(messages, latest, unread_after, window, cx);
                    }
                    Err(err) if refreshing => eprintln!("refreshing a conversation failed: {err}"),
                    Err(err) => this.messages_error = Some(err),
                }
                this.ack_open_conversation(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn show_opening_page(
        &mut self,
        messages: Vec<discord::Message>,
        latest: bool,
        unread_after: Option<Id<MessageMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let full = messages.len() == discord::MESSAGE_PAGE_SIZE;
        self.reached_oldest = latest && !full;
        self.reached_newest = latest || !full;
        self.messages = messages;
        self.messages_channel = self.selected_channel;
        // An older page asked for of what the cache showed would land against
        // the wrong messages.
        self.older_loading = false;
        self.request_author_members();
        // Resetting also snaps the bottom-aligned list to the newest message.
        self.messages_list.reset(self.messages.len());
        // Opened just past the last read message, the read ones above it give
        // the divider something to sit under.
        if !latest {
            self.load_older_messages(cx);
        }
        if unread_after.is_some() {
            self.scroll_to_unread(window, cx);
        }
    }

    /// Puts the first unread message at the top of the view.
    ///
    /// The list won't scroll past its end, so if the unread all fit it lands at the
    /// bottom, already read. That's only known after layout, hence the wait;
    /// next-frame callbacks run before the frame they're queued for is drawn, hence two.
    fn scroll_to_unread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = match self.unread_divider_index() {
            Some(ix) => ix,
            None if !self.reached_newest => 0,
            None => return,
        };
        self.messages_list.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(0.),
        });
        self.at_bottom = false;

        let screen = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                let _ = screen.update(cx, |this, cx| {
                    let list = &this.messages_list;
                    // Pinned to the bottom, the list reports its top as past
                    // the last item.
                    if !this.at_bottom && list.logical_scroll_top().item_ix >= list.item_count() {
                        this.at_bottom = true;
                        this.ack_open_conversation(cx);
                    }
                });
            });
        });
    }

    pub(in crate::screens::home) fn reset_conversation(
        &mut self,
        placeholder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(channel_id) = self.messages_channel.take()
            && self.messages_error.is_none()
            && self.reached_newest
        {
            let messages = std::mem::take(&mut self.messages);
            self.history_cache
                .store(channel_id, messages, self.reached_oldest);
        }
        self.messages.clear();
        self.messages_list.reset(0);
        self.messages_generation += 1;
        self.reactors.clear();
        // Release the previous conversation's decoded images; the next one repopulates
        // the cache.
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.release_gifs(window, cx);
        self.stop_video(window, cx);
        self.messages_error = None;
        self.older_loading = false;
        self.reached_oldest = false;
        self.newer_loading = false;
        self.reached_newest = true;
        // Starts pinned to the newest message, so live messages follow until the user
        // scrolls up.
        self.at_bottom = true;
        self.send_error = None;
        self.replying_to = None;
        self.editing = None;
        self.pins_popup = None;
        self.revealed_spoilers.clear();
        self.text_selection = None;
        self.text_copies.get_mut().clear();
        self.flashed_message = None;
        self.flash_timer = None;
        self.unread_marker = None;
        self.marked_unread = false;
        for attachment in self.pending_attachments.drain(..) {
            attachment.release_preview(cx);
        }
        let placeholder = placeholder.unwrap_or_else(|| "Send a message".into());
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

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
        self.older_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_messages(channel_id, MessagePage::Before(oldest_id)).await;
            let _ = this.update(cx, |this, cx| {
                // Drop the response if the channel changed or messages reloaded meanwhile.
                if this.selected_channel != Some(channel_id)
                    || this.messages.first().map(|message| message.id) != Some(oldest_id)
                {
                    return;
                }
                this.older_loading = false;
                // A failed fetch leaves the messages on screen; it retries the
                // next time the user scrolls near the top.
                if let Ok(older) = result {
                    if older.len() < discord::MESSAGE_PAGE_SIZE {
                        this.reached_oldest = true;
                    }
                    let count = older.len();
                    if count > 0 {
                        this.messages.splice(0..0, older);
                        this.request_author_members();
                        this.messages_list.splice(0..0, count);
                        // The divider may now belong above what was the
                        // oldest row, which was measured without it.
                        if this.unread_divider_index() == Some(count) {
                            this.messages_list.splice(count..count + 1, 1);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn load_newer_messages(&mut self, cx: &mut Context<Self>) {
        if self.messages_loading || self.newer_loading || self.reached_newest {
            return;
        }
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(newest_id) = self.messages.last().map(|message| message.id) else {
            return;
        };
        self.newer_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_messages(channel_id, MessagePage::After(newest_id)).await;
            let _ = this.update(cx, |this, cx| {
                if this.selected_channel != Some(channel_id)
                    || this.messages.last().map(|message| message.id) != Some(newest_id)
                {
                    return;
                }
                this.newer_loading = false;
                // A failed fetch retries the next time the user scrolls near
                // the end.
                if let Ok(newer) = result {
                    if newer.len() < discord::MESSAGE_PAGE_SIZE {
                        this.reached_newest = true;
                    }
                    let count = newer.len();
                    if count > 0 {
                        // Scrolled to the very end, the list is pinned to the bottom and would ride
                        // new
                        // rows down past all of them. Pin it a viewport up from the end instead.
                        let list = &this.messages_list;
                        if list.logical_scroll_top().item_ix >= list.item_count() {
                            list.scroll_by(-list.viewport_bounds().size.height);
                        }
                        let ix = this.messages.len();
                        this.messages.extend(newer);
                        this.request_author_members();
                        this.splice_messages(ix..ix, count);
                        this.at_bottom = false;
                    }
                }
                this.ack_open_conversation(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
