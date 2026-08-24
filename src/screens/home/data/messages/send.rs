//! Sending what's in the composer to the open conversation.

use gpui::*;

use crate::discord;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(in crate::screens::home) fn send_current_message(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // Guard the send path too: the composer is replaced by a notice on
        // channels the user can't post in, but Enter could still reach here.
        if self.view != View::DirectMessages
            && !self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_send)
        {
            return;
        }

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
            .map(|attachment| {
                attachment.release_preview(cx);
                (attachment.filename, attachment.data.bytes().to_vec())
            })
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
}
