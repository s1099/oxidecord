//! Deleting a message, and copying a link to one.
//!
//! There is no `MESSAGE_DELETE` handling on the gateway side yet, so the row is
//! dropped locally as soon as the request is sent and put back if it fails.

use gpui::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// Whether the current user is allowed to delete `message`: their own
    /// message anywhere, or anyone's in a channel they manage.
    pub(in crate::screens::home) fn can_delete_message(&self, message: &discord::Message) -> bool {
        let own = self
            .current_user
            .as_ref()
            .is_some_and(|user| user.id == message.author_id);
        own || self
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
        let Some(token) = discord::load_token() else {
            self.send_error = Some("No token found. Please log in first.".into());
            cx.notify();
            return;
        };
        let Some(ix) = self
            .messages
            .iter()
            .position(|message| message.id == message_id)
        else {
            return;
        };

        // Drop the row now; the request usually succeeds, and waiting on it
        // would leave the message sitting there after the click.
        let removed = self.messages.remove(ix);
        self.messages_list.splice(ix..ix + 1, 0);
        cx.notify();

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::delete_message(token, channel_id, message_id, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(Err(err)) = rx.await else {
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
                this.messages_list.splice(ix..ix, 1);
                this.send_error = Some(err);
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts a `discord.com` link to the message on the clipboard, the same URL
    /// the official client's "Copy Message Link" produces.
    pub(in crate::screens::home) fn copy_message_link(
        &self,
        message_id: Id<MessageMarker>,
        cx: &mut App,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // DMs have no guild, and use the literal `@me` in its place.
        let guild = match self.view {
            View::DirectMessages => "@me".to_string(),
            View::Guild => match self.selected_guild {
                Some(guild_id) => guild_id.to_string(),
                None => return,
            },
        };

        cx.write_to_clipboard(ClipboardItem::new_string(format!(
            "https://discord.com/channels/{guild}/{channel_id}/{message_id}"
        )));
    }
}
