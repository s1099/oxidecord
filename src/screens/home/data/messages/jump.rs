//! Jumping to a message: a reply's original, the message a composer reply is
//! for, or a link's target. History around it loads if needed, and it lights up
//! once on screen, as in Discord.

use std::time::Duration;

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord::{self, ChannelHome, MessageLink, MessagePage};
use crate::screens::home::HomeScreen;

const FLASH: Duration = Duration::from_secs(2);

impl HomeScreen {
    /// Opens a message or channel link. False for a guild the user isn't in,
    /// which only the browser can show.
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
            if let Some(message_id) = message_id {
                self.jump_to_message(message_id, window, cx);
            }
            return true;
        }
        let pending_jump = message_id.map(|message_id| (channel_id, message_id));
        match guild_id {
            Some(guild_id) => {
                if !self.guilds.iter().any(|guild| guild.id == guild_id) {
                    return false;
                }
                // Not one of the guild's channels, it's a thread, which may need fetching.
                let listed = self
                    .guild_data
                    .channels
                    .get(&guild_id)
                    .is_none_or(|channels| channels.iter().any(|c| c.id == channel_id));
                if !listed {
                    self.pending_jump = pending_jump;
                    self.open_thread_or(channel_id, Some(link.web_url()), window, cx);
                    return true;
                }
                // Opening the guild reopens its last channel, so the link's
                // is opened even when the gateway has yet to send the list.
                self.last_guild_channel.insert(guild_id, channel_id);
                self.pending_jump = pending_jump;
                self.select_guild(guild_id, window, cx);
                // Already open, or the link is to a voice channel's chat,
                // which reopening a guild never opens.
                if !is_open(self) && self.channel_info(channel_id).is_some() {
                    self.pending_jump = pending_jump;
                    if is_voice(self) {
                        self.open_voice_chat(channel_id, window, cx);
                    } else {
                        self.select_channel(channel_id, window, cx);
                    }
                }
            }
            None if self.dms.list.iter().any(|dm| dm.id == channel_id) => {
                self.pending_jump = pending_jump;
                self.open_dm(channel_id, window, cx);
            }
            // `@me` links are also made to guild channels, and to DMs that
            // aren't open, so the channel itself says where it lives.
            None => {
                if let Some(guild_id) = self.known_channel_guild(channel_id) {
                    let link = MessageLink {
                        guild_id: Some(guild_id),
                        ..link
                    };
                    return self.open_message_link(link, window, cx);
                }
                cx.spawn_in(window, async move |this, cx| {
                    let home = discord::fetch_channel_home(channel_id).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        let opened = match home {
                            Ok(Some(ChannelHome::Guild(guild_id))) => {
                                let link = MessageLink {
                                    guild_id: Some(guild_id),
                                    ..link
                                };
                                this.open_message_link(link, window, cx)
                            }
                            Ok(Some(ChannelHome::Dm(dm))) => {
                                this.dms.list.insert(0, dm);
                                this.pending_jump = pending_jump;
                                this.open_dm(channel_id, window, cx);
                                cx.notify();
                                true
                            }
                            Ok(None) | Err(_) => false,
                        };
                        if !opened {
                            cx.open_url(&link.web_url());
                        }
                    });
                })
                .detach();
            }
        }
        cx.notify();
        true
    }

    fn known_channel_guild(&self, channel_id: Id<ChannelMarker>) -> Option<Id<GuildMarker>> {
        self.guild_data
            .channels
            .iter()
            .find(|(_, channels)| channels.iter().any(|c| c.id == channel_id))
            .map(|(&guild_id, _)| guild_id)
            .or_else(|| self.threads.get(channel_id)?.thread.guild_id)
    }

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

    /// Skipped if already all in sight. Centring needs the row's height, known only
    /// after layout, so it waits; next-frame callbacks run before the frame they're
    /// queued for is drawn, hence two.
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
