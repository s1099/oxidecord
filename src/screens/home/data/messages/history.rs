//! Opening a conversation, at the first unread message when there is one, and
//! paging through its history either way from there.

use std::time::Duration;

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, MessagePage};
use crate::screens::home::history_cache::merge_latest;
use crate::screens::home::{HomeScreen, MessageTarget, View};

/// How long the tint marking where a jump landed stays on before clearing
/// itself, like Discord's does.
const HIGHLIGHT_FLASH: Duration = Duration::from_secs(3);

impl HomeScreen {
    pub(in crate::screens::home) fn select_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click on another conversation cancels a jump still in flight.
        self.pending_message_target = None;
        self.apply_channel_selection(channel_id, window, cx);
    }

    /// Selects `channel_id` and loads its messages, leaving any pending jump
    /// alone: a jump sets its target first and comes through here.
    pub(in crate::screens::home) fn apply_channel_selection(
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
        if self.view == View::DirectMessages
            && let Some(gateway) = &self.gateway
        {
            gateway.request_call_connect(channel_id);
        }
        self.load_messages(window, cx);
    }

    /// Opens the conversation a reply quote, forward header, or message link
    /// points at, scrolled to that message and with it flashed to mark the
    /// landing. When the conversation is already open and holds the message,
    /// the history is kept and only the scroll moves.
    pub(in crate::screens::home) fn jump_to_message(
        &mut self,
        target: MessageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The conversation is already open and holds the message: scrolling to
        // it keeps the history the user has around it, which a fetch-around
        // would throw away for a fresh page.
        let same_conversation = match self.view {
            View::DirectMessages => target.guild_id.is_none(),
            View::Guild => target.guild_id == self.selected_guild,
        };
        if !self.messages_loading
            && self.selected_channel == Some(target.channel_id)
            && same_conversation
            && let Some(index) = self
                .messages
                .iter()
                .position(|message| message.id == target.message_id)
        {
            self.messages_list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
            // Landed on the newest message, the list sits pinned to the bottom
            // and should keep following the conversation.
            self.at_bottom = index + 1 == self.messages.len() && self.reached_newest;
            self.flash_highlight(target.message_id, cx);
            cx.notify();
            return;
        }
        match target.guild_id {
            Some(guild_id) => {
                self.pending_message_target = Some(target);
                self.open_guild_channel_at(guild_id, Some(target.channel_id), window, cx);
            }
            None => {
                if self.view != View::DirectMessages {
                    self.open_direct_messages(window, cx);
                }
                self.pending_message_target = Some(target);
                self.apply_channel_selection(target.channel_id, window, cx);
            }
        }
        // The channel was open already, so opening it did nothing: load for
        // the pending target here, and let `load_messages` take it from there.
        if self.selected_channel == Some(target.channel_id)
            && self.pending_message_target == Some(target)
        {
            self.load_messages(window, cx);
        }
    }

    /// Tints the message a jump landed on, clearing itself a moment later so
    /// the mark doesn't outlive the jump that made it. The serial tells a
    /// late clear of an older flash apart from a newer flash of the same
    /// message, which restarted the timer.
    fn flash_highlight(&mut self, message_id: Id<MessageMarker>, cx: &mut Context<Self>) {
        self.highlighted_message = Some(message_id);
        self.highlight_serial = self.highlight_serial.wrapping_add(1);
        let serial = self.highlight_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIGHLIGHT_FLASH).await;
            let _ = this.update(cx, |this, cx| {
                if this.highlight_serial == serial {
                    this.highlighted_message = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn load_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let placeholder = match self.view {
            View::DirectMessages => self
                .selected_dm_info()
                .map(|dm| format!("Message @{}", dm.name)),
            View::Guild => self
                .selected_channel_info()
                .map(|channel| format!("Message #{}", channel.name)),
        };
        // Where the user had read up to, taken before opening acks it.
        let marker = self
            .unreads
            .is_unread(channel_id)
            .then(|| self.unreads.acked(channel_id))
            .flatten();
        self.reset_conversation(placeholder, window, cx);
        self.unread_marker = marker;
        // Opened before, it's drawn from the cache while the newest page is
        // fetched, which is merged in when it lands. A jump skips the cache:
        // its fetch-around replaces the list wholesale.
        let target = self.pending_message_target.take().filter(|target| {
            target.channel_id == channel_id
                && match self.view {
                    View::DirectMessages => target.guild_id.is_none(),
                    View::Guild => target.guild_id == self.selected_guild,
                }
        });
        match target {
            Some(_) => self.messages_loading = true,
            None => match self.history_cache.take(channel_id) {
                Some(cached) => {
                    self.messages = cached.messages;
                    self.reached_oldest = cached.reached_oldest;
                    self.messages_channel = Some(channel_id);
                    self.request_author_members();
                    self.messages_list.reset(self.messages.len());
                }
                None => self.messages_loading = true,
            },
        }
        if let Some(target) = target {
            self.fetch_message_target(target, window, cx);
        } else {
            self.fetch_opening_page(channel_id, marker, window, cx);
        }
    }

    /// Leaves history opened further back for the newest page, as Discord's
    /// "Jump To Present" does.
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
        self.message_load_generation = self.message_load_generation.wrapping_add(1);
        self.highlighted_message = None;
        self.messages.clear();
        self.messages_channel = None;
        self.messages_list.reset(0);
        self.reactors.clear();
        self.older_loading = false;
        self.newer_loading = false;
        self.at_bottom = true;
        self.messages_loading = true;
        self.fetch_opening_page(channel_id, None, window, cx);
        cx.notify();
    }

    /// Fetches the page a conversation opens on: the newest, scrolled to the
    /// "new" divider when `unread_after` — the last message read — falls in
    /// it. When it falls further back, the unread run is longer than a page,
    /// and the conversation opens where the user left off instead, as
    /// Discord's does, to be paged forward from there.
    fn fetch_opening_page(
        &mut self,
        channel_id: Id<ChannelMarker>,
        unread_after: Option<Id<MessageMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.message_load_generation;
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
                // The user may have clicked another channel while this request
                // was in flight; drop the stale response.
                if this.selected_channel != Some(channel_id)
                    || this.message_load_generation != generation
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
                    // What the cache showed stays up.
                    Err(err) if refreshing => eprintln!("refreshing a conversation failed: {err}"),
                    Err(err) => this.messages_error = Some(err),
                }
                this.ack_open_conversation(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetches a page centred on a jump's target and opens the conversation
    /// scrolled to it, replacing whatever the cache showed wholesale.
    fn fetch_message_target(
        &mut self,
        target: MessageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = self.message_load_generation;
        cx.spawn_in(window, async move |this, cx| {
            let result =
                discord::fetch_messages(target.channel_id, MessagePage::Around(target.message_id))
                    .await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.selected_channel != Some(target.channel_id)
                    || this.message_load_generation != generation
                {
                    return;
                }
                this.messages_loading = false;
                match result {
                    Ok(messages) => {
                        if let Some(index) = messages
                            .iter()
                            .position(|message| message.id == target.message_id)
                        {
                            let complete = messages.len() < discord::MESSAGE_PAGE_SIZE;
                            this.reached_oldest = complete;
                            this.reached_newest = complete;
                            // The jumped-to message may itself be the newest,
                            // in which case the list has landed back on the
                            // bottom and should keep following.
                            this.at_bottom = index + 1 == messages.len() && complete;
                            this.messages = messages;
                            this.messages_channel = Some(target.channel_id);
                            this.request_author_members();
                            this.messages_list.reset(this.messages.len());
                            this.messages_list.scroll_to(ListOffset {
                                item_ix: index,
                                offset_in_item: px(0.),
                            });
                            this.flash_highlight(target.message_id, cx);
                        } else {
                            this.messages_error = Some(
                                "That message is unavailable, or you cannot read its history."
                                    .into(),
                            );
                        }
                    }
                    Err(error) => this.messages_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Replaces the conversation with the page it opens on.
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
    /// The list stops short of scrolling past its end, so when the unread
    /// all fit on screen this lands at the bottom after all, and they've been
    /// read. That's only known once the list has been laid out, so the check
    /// waits for it: next-frame callbacks run before the frame they're queued
    /// for is drawn, hence two of them.
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
        let generation = self.message_load_generation;
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                let _ = screen.update(cx, |this, cx| {
                    if this.message_load_generation != generation {
                        return;
                    }
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

    /// Empties the message pane and everything tied to the conversation that
    /// was open, ready for the next one (or none).
    pub(in crate::screens::home) fn reset_conversation(
        &mut self,
        placeholder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message_load_generation = self.message_load_generation.wrapping_add(1);
        self.highlighted_message = None;
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
        self.reactors.clear();
        // Release the previous conversation's decoded images instead of letting
        // them accumulate; the next one repopulates the cache as it renders.
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.stop_video(window, cx);
        self.messages_error = None;
        self.older_loading = false;
        self.reached_oldest = false;
        self.newer_loading = false;
        self.reached_newest = true;
        // A freshly opened conversation starts pinned to its newest message, so
        // live messages should follow along until the user scrolls up.
        self.at_bottom = true;
        self.send_error = None;
        self.replying_to = None;
        self.editing = None;
        self.pins_popup = None;
        self.revealed_spoilers.clear();
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
        self.older_loading = true;
        let generation = self.message_load_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_messages(channel_id, MessagePage::Before(oldest_id)).await;
            let _ = this.update(cx, |this, cx| {
                // Drop the response if the channel changed or the messages
                // were reloaded while this request was in flight.
                if this.selected_channel != Some(channel_id)
                    || this.message_load_generation != generation
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

    /// Fetches the page newer than the newest loaded message and appends it,
    /// while the conversation is open further back than its present.
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
        let generation = self.message_load_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_messages(channel_id, MessagePage::After(newest_id)).await;
            let _ = this.update(cx, |this, cx| {
                if this.selected_channel != Some(channel_id)
                    || this.message_load_generation != generation
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
                        // Scrolled right to the end, the list is pinned to its
                        // bottom and would ride the new rows down past every
                        // one of them. Pin it where it stands instead: a
                        // viewport up from the end.
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
