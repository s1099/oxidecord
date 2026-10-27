//! Deleting a message, taking deletions made elsewhere, and copying a link to one.
//!
//! A local deletion drops the row when the request is sent and restores it on
//! failure; the gateway's echo then finds nothing to remove.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// Own messages anywhere, anyone's in a channel they manage.
    pub(in crate::screens::home) fn can_delete_message(&self, message: &discord::Message) -> bool {
        self.is_own_message(message)
            || self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_manage_messages)
    }

    pub(in crate::screens::home) fn delete_message(
        &mut self,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        else {
            return;
        };

        // Dropped now: waiting on the request would leave the message sitting there
        // after the click.
        let removed = self.messages.remove(ix);
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.message_id == message_id)
        {
            self.editing = None;
        }
        self.splice_messages(ix..ix + 1, 0);
        cx.notify();

        cx.spawn(async move |this, cx| {
            let Err(err) = discord::delete_message(channel_id, message_id).await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Nothing to restore if the user switched channels meanwhile:
                // the list they're looking at isn't the one we took it from.
                if this.selected_channel != Some(channel_id) {
                    return;
                }
                let ix = ix.min(this.messages.len());
                this.messages.insert(ix, removed);
                this.splice_messages(ix..ix, 1);
                this.send_error = Some(err);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn handle_message_delete(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_ids: &[Id<MessageMarker>],
        cx: &mut Context<Self>,
    ) {
        // Mid-load, the history about to land replaces the list anyway.
        if self.selected_channel != Some(channel_id) || self.messages_loading {
            return;
        }
        let mut changed = false;
        for &message_id in message_ids {
            let Some(ix) = self
                .messages
                .iter()
                .position(|message| message.id == message_id)
            else {
                continue;
            };
            self.messages.remove(ix);
            self.splice_messages(ix..ix + 1, 0);
            changed = true;
        }
        for message in &mut self.messages {
            changed |= message.forget_deleted_reply(message_ids);
        }
        if !changed {
            return;
        }

        // An edit or a reply aimed at a message that's gone could only fail.
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| message_ids.contains(&editing.message_id))
        {
            self.editing = None;
        }
        if self
            .replying_to
            .as_ref()
            .is_some_and(|reply| message_ids.contains(&reply.message_id))
        {
            self.replying_to = None;
        }
        cx.notify();
    }

    /// The same URL as the official client's "Copy Message Link".
    pub(in crate::screens::home) fn copy_message_link(
        &self,
        message_id: Id<MessageMarker>,
        cx: &mut App,
    ) {
        if let Some(link) = self.message_link(message_id) {
            cx.write_to_clipboard(ClipboardItem::new_string(link));
        }
    }

    /// A link to a message in the open channel.
    pub(in crate::screens::home) fn message_link(
        &self,
        message_id: Id<MessageMarker>,
    ) -> Option<String> {
        let channel_id = self.selected_channel?;
        // DMs have no guild, and use the literal `@me` in its place.
        let guild = match self.view {
            View::DirectMessages => "@me".to_string(),
            View::Guild => self.selected_guild?.to_string(),
        };
        Some(format!(
            "https://discord.com/channels/{guild}/{channel_id}/{message_id}"
        ))
    }
}
