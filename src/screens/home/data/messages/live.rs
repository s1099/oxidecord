//! The gateway connection that keeps the open conversation live.

use gpui::*;

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Opens the gateway connection and pumps live `MESSAGE_CREATE` events
    /// onto the gpui foreground, where they update the open conversation.
    pub(in crate::screens::home) fn start_gateway(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::mpsc::unbounded::<discord::IncomingMessage>();
        discord::connect_gateway(token, move |incoming| {
            // Returns whether the foreground receiver is still around; once it
            // isn't (the screen was dropped), the gateway loop stops.
            tx.unbounded_send(incoming).is_ok()
        });

        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;

            let mut rx = rx;
            while let Some(incoming) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.handle_incoming_message(incoming, cx))
                    .is_err()
                {
                    // The entity is gone; stop draining so the sender closes.
                    break;
                }
            }
        })
        .detach();
    }

    /// Appends a live message to the open conversation, if it belongs there.
    fn handle_incoming_message(
        &mut self,
        incoming: discord::IncomingMessage,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel != Some(incoming.channel_id) {
            return;
        }
        // The history for this channel is still loading and will replace the
        // whole list when it lands (and include this message), so skip it now
        // to avoid a desync between `messages` and the list state.
        if self.messages_loading {
            return;
        }
        // Ignore duplicates: the echo of a message we just sent ourselves, or a
        // repeated dispatch.
        if self
            .messages
            .iter()
            .any(|message| message.id == incoming.message.id)
        {
            return;
        }

        let ix = self.messages.len();
        self.messages.push(incoming.message);
        self.messages_list.splice(ix..ix, 1);
        // Follow the conversation only when the newest message was already in
        // view; if the user has scrolled up to read history, leave them there.
        if self.at_bottom {
            self.messages_list.scroll_to(ListOffset {
                item_ix: ix + 1,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }
}
