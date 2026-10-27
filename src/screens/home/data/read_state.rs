//! Keeping track of what's been read: acking the open conversation as the
//! user reads it, marking a message unread or the whole conversation read,
//! and the "new" divider that shows where they'd read up to.

use std::time::Duration;

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

/// Batched: following a busy channel live would otherwise send one ack per message.
const ACK_DELAY: Duration = Duration::from_secs(2);

impl HomeScreen {
    /// Once the user can see it: loaded, scrolled to the bottom, and not marked
    /// unread (which holds until they leave).
    pub(in crate::screens::home) fn ack_open_conversation(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // A voice channel open without its chat has no conversation loaded,
        // or still holds the one open before it.
        if self.messages_channel != Some(channel_id)
            || self.messages_loading
            || self.messages_error.is_some()
            || !self.at_bottom
            || !self.reached_newest
            || self.marked_unread
        {
            return;
        }
        let Some(newest) = self.newest_sent_message() else {
            return;
        };
        self.unreads.note_latest(channel_id, newest);
        self.queue_ack(channel_id, newest, cx);
    }

    /// Called every frame: the list's scroll handler fires only for the wheel and
    /// reports where the list was before the event moved it, and smooth scrolling
    /// glides on without it, so the bottom would be noticed a tick late.
    pub(in crate::screens::home) fn sync_at_bottom(&mut self, cx: &mut Context<Self>) {
        let list = &self.messages_list;
        // Pinned to the bottom, the list reports its top as past the last
        // item.
        let at_bottom = list.logical_scroll_top().item_ix >= list.item_count();
        if at_bottom == self.at_bottom {
            return;
        }
        self.at_bottom = at_bottom;
        if at_bottom {
            self.ack_open_conversation(cx);
        }
    }

    /// As escape and the "new messages" bar do in Discord. Also undoes marking it
    /// unread; the divider goes.
    pub(in crate::screens::home) fn mark_conversation_read(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel.is_none() {
            return;
        }
        self.marked_unread = false;
        self.set_unread_marker(None);
        if self.reached_newest {
            self.messages_list.scroll_to(ListOffset {
                item_ix: self.messages.len(),
                offset_in_item: px(0.),
            });
            self.at_bottom = true;
            self.ack_open_conversation(cx);
        } else {
            // Acked when the newest page lands, which opens at the bottom.
            self.jump_to_present(window, cx);
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn jump_to_first_unread(&mut self, cx: &mut Context<Self>) {
        let ix = self.unread_divider_index().unwrap_or(0);
        self.messages_list.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    /// `None` while nothing is unread or the conversation hasn't loaded.
    pub(in crate::screens::home) fn unread_summary(&self) -> Option<(usize, bool, String)> {
        let channel_id = self.selected_channel?;
        if self.messages_loading || !self.unreads.is_unread(channel_id) {
            return None;
        }
        let acked = self.unreads.acked(channel_id)?;
        let ix = self.messages.partition_point(|message| message.id <= acked);
        let first = self.messages.get(ix)?;
        let count = self.messages[ix..]
            .iter()
            .filter(|message| message.delivery == discord::Delivery::Sent)
            .count();
        // Opened further back than the present, there are more past the end.
        Some((count, !self.reached_newest, first.timestamp_label()))
    }

    /// Discord's counting rules: a ping by name, of everyone, or of one of the user's
    /// roles, unless guild settings suppress those; in a DM every message unless
    /// muted. The user's own never count.
    pub(in crate::screens::home) fn pings_me(
        &self,
        channel_id: Id<ChannelMarker>,
        guild_id: Option<Id<GuildMarker>>,
        message: &discord::Message,
    ) -> bool {
        let Some(self_id) = self.self_user_id else {
            return false;
        };
        if message.author_id == self_id {
            return false;
        }
        if message.mentions.iter().any(|user| user.id == self_id) {
            return true;
        }
        let Some(guild_id) = guild_id else {
            return !self.unreads.is_muted(channel_id, None);
        };
        let (everyone, roles) = self.unreads.pinged_by(guild_id);
        (everyone && message.mention_everyone)
            || (roles && self.mentions_self_role(message, guild_id))
    }

    /// Sent after a moment, batched with others that pile up meanwhile.
    fn queue_ack(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        if !self.unreads.ack(channel_id, message_id) {
            return;
        }
        cx.notify();
        self.pending_acks.insert(channel_id, message_id);
        if self.ack_flush.is_some() {
            return;
        }
        self.ack_flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ACK_DELAY).await;
            let Ok(acks) = this.update(cx, |this, _| {
                this.ack_flush = None;
                std::mem::take(&mut this.pending_acks)
            }) else {
                return;
            };
            for (channel_id, message_id) in acks {
                if let Err(err) = discord::ack_message(channel_id, message_id, false, 0).await {
                    eprintln!("acking channel {channel_id} failed: {err}");
                }
            }
        }));
    }

    /// As alt-click does in Discord. The divider moves above the message and no ack
    /// is sent until the user leaves.
    pub(in crate::screens::home) fn mark_unread(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // An ack needn't name a real message, so the one just before is
        // simply the snowflake before it.
        let Some(before) = Id::new_checked(message_id.get() - 1) else {
            return;
        };
        if let Some(newest) = self.newest_sent_message() {
            self.unreads.note_latest(channel_id, newest);
        }
        // Discord leaves the client to say how many pings are past the ack,
        // and only the loaded messages can be counted. In an unmuted DM
        // that's every message, the user's own included, as Discord's
        // client counts it.
        let guild_id = self.selected_guild.filter(|_| self.view == View::Guild);
        let every_message = guild_id.is_none() && !self.unreads.is_muted(channel_id, None);
        let ix = self
            .messages
            .partition_point(|message| message.id <= before);
        let mentions = self.messages[ix..]
            .iter()
            .filter(|message| message.delivery == discord::Delivery::Sent)
            .filter(|message| every_message || self.pings_me(channel_id, guild_id, message))
            .count() as u32;
        self.unreads.set_acked(channel_id, before, mentions);
        self.pending_acks.remove(&channel_id);
        self.marked_unread = true;
        self.set_unread_marker(Some(before));
        cx.notify();

        cx.spawn(async move |_, _| {
            if let Err(err) = discord::ack_message(channel_id, before, true, mentions).await {
                eprintln!("marking channel {channel_id} unread failed: {err}");
            }
        })
        .detach();
    }

    /// Re-measures the rows it leaves and lands on: the list caches row heights and
    /// the divider is part of its row.
    pub(in crate::screens::home) fn set_unread_marker(
        &mut self,
        marker: Option<Id<MessageMarker>>,
    ) {
        let old = self.unread_divider_index();
        self.unread_marker = marker;
        let new = self.unread_divider_index();
        for ix in [old, new].into_iter().flatten() {
            self.messages_list.splice(ix..ix + 1, 1);
        }
    }

    /// The first message past the marker, only if the one before it is loaded too (or
    /// there is none); otherwise the marker is further back than the list goes.
    pub(in crate::screens::home) fn unread_divider_index(&self) -> Option<usize> {
        let marker = self.unread_marker?;
        let ix = self
            .messages
            .partition_point(|message| message.id <= marker);
        (ix < self.messages.len() && (ix > 0 || self.reached_oldest)).then_some(ix)
    }

    /// A pending message's id is only its nonce.
    fn newest_sent_message(&self) -> Option<Id<MessageMarker>> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.delivery == discord::Delivery::Sent)
            .map(|message| message.id)
    }
}
