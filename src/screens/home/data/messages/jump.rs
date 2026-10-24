//! Jumping to a message: a reply's original, the message a reply in the
//! composer is for, or a message link's target. History around it is loaded
//! when it isn't already, and it lights up once it's on screen, as Discord's
//! does.

use std::time::Duration;

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, MessageLink, MessagePage};
use crate::screens::home::HomeScreen;

/// How long a message jumped to stays lit.
const FLASH: Duration = Duration::from_secs(2);

impl HomeScreen {
    /// Opens a message link's conversation at its message. Returns false for
    /// one in a guild the user isn't in, which only the browser can show.
    pub(in crate::screens::home) fn open_message_link(
        &mut self,
        link: MessageLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let MessageLink {
            guild_id,
            channel_id,
            message_id,
        } = link;
        let is_voice = |this: &Self| {
            this.channel_info(channel_id)
                .is_some_and(|channel| channel.kind.is_voice())
        };
        // A voice channel's conversation is only open alongside its chat.
        let is_open = |this: &Self| {
            this.selected_channel == Some(channel_id) && (!is_voice(this) || this.voice_chat_open)
        };
        if is_open(self) {
            self.jump_to_message(message_id, window, cx);
            return true;
        }
        match guild_id {
            Some(guild_id) => {
                if !self.guilds.iter().any(|guild| guild.id == guild_id) {
                    return false;
                }
                // Opening the guild reopens its last channel, so the link's
                // is opened even when the gateway has yet to send the list.
                self.last_guild_channel.insert(guild_id, channel_id);
                self.pending_jump = Some((channel_id, message_id));
                self.select_guild(guild_id, window, cx);
                // Already open, or the link is to a voice channel's chat,
                // which reopening a guild never opens.
                if !is_open(self) && self.channel_info(channel_id).is_some() {
                    self.pending_jump = Some((channel_id, message_id));
                    if is_voice(self) {
                        self.open_voice_chat(channel_id, window, cx);
                    } else {
                        self.select_channel(channel_id, window, cx);
                    }
                }
            }
            None => {
                self.pending_jump = Some((channel_id, message_id));
                self.open_direct_messages(window, cx);
                self.select_channel(channel_id, window, cx);
            }
        }
        cx.notify();
        true
    }

    /// Scrolls the open conversation to `message_id`, loading the history
    /// around it first when it isn't loaded.
    pub(in crate::screens::home) fn jump_to_message(
        &mut self,
        message_id: Id<MessageMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        if self.messages_channel == Some(channel_id)
            && let Some(ix) = self.message_index(message_id)
        {
            self.reveal_message(message_id, ix, window, cx);
            return;
        }
        self.load_around(channel_id, message_id, window, cx);
    }

    /// Replaces the conversation with the page around `message_id`, as far
    /// back as it is, to be paged either way from there.
    pub(in crate::screens::home) fn load_around(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.messages.clear();
        self.messages_channel = None;
        self.messages_list.reset(0);
        self.messages_generation += 1;
        self.reactors.clear();
        self.messages_error = None;
        self.older_loading = false;
        self.newer_loading = false;
        self.at_bottom = false;
        self.messages_loading = true;
        cx.notify();

        let generation = self.messages_generation;
        cx.spawn_in(window, async move |this, cx| {
            let result = discord::fetch_messages(channel_id, MessagePage::Around(message_id)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.selected_channel != Some(channel_id)
                    || this.messages_generation != generation
                {
                    return;
                }
                this.messages_loading = false;
                match result {
                    Ok(messages) => {
                        // A page around a message doesn't say whether it
                        // reaches either end. The newest is known from the
                        // read states; the oldest is found by paging there.
                        this.reached_newest =
                            match (messages.last(), this.unreads.latest(channel_id)) {
                                (None, _) => true,
                                (Some(last), Some(latest)) => last.id >= latest,
                                (Some(_), None) => false,
                            };
                        this.reached_oldest = messages.is_empty();
                        this.messages = messages;
                        this.messages_channel = Some(channel_id);
                        this.request_author_members();
                        this.messages_list.reset(this.messages.len());
                        // A deleted message is gone from the page; it opens
                        // where it was, with nothing to light up.
                        match this.message_index(message_id) {
                            Some(ix) => this.reveal_message(message_id, ix, window, cx),
                            None => {
                                let ix = this.messages.partition_point(|m| m.id < message_id);
                                this.messages_list.scroll_to(ListOffset {
                                    item_ix: ix,
                                    offset_in_item: px(0.),
                                });
                            }
                        }
                    }
                    Err(err) => this.messages_error = Some(err),
                }
                this.ack_open_conversation(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn message_index(&self, message_id: Id<MessageMarker>) -> Option<usize> {
        self.messages
            .iter()
            .rposition(|message| message.id == message_id)
    }

    /// Brings the message at `ix` into the middle of the view, unless it's
    /// already all in sight, and lights it up.
    ///
    /// Centring needs the message's height, which is only known once the list
    /// has laid it out, so it waits for that: next-frame callbacks run before
    /// the frame they're queued for is drawn, hence two of them.
    fn reveal_message(
        &mut self,
        message_id: Id<MessageMarker>,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flash_message(message_id, cx);
        let list = &self.messages_list;
        let viewport = list.viewport_bounds();
        if let Some(bounds) = list.bounds_for_item(ix)
            && bounds.top() >= viewport.top()
            && bounds.bottom() <= viewport.bottom()
        {
            return;
        }
        list.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(0.),
        });
        self.at_bottom = false;

        let screen = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                let _ = screen.update(cx, |this, cx| {
                    let list = &this.messages_list;
                    // Too near the end to reach the top, the list stops at
                    // its bottom, and is pinned there: the newest is in view.
                    if list.logical_scroll_top().item_ix >= list.item_count() {
                        this.at_bottom = true;
                        this.ack_open_conversation(cx);
                    } else if let Some(bounds) = this
                        .message_index(message_id)
                        .and_then(|ix| list.bounds_for_item(ix))
                    {
                        let above = (list.viewport_bounds().size.height - bounds.size.height) / 2.;
                        if above > px(0.) {
                            list.scroll_by(-above);
                        }
                    }
                    cx.notify();
                });
            });
        });
    }

    fn flash_message(&mut self, message_id: Id<MessageMarker>, cx: &mut Context<Self>) {
        self.flashed_message = Some(message_id);
        self.flash_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            let _ = this.update(cx, |this, cx| {
                this.flashed_message = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
}
