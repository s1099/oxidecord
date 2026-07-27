//! Loading the direct-message list and switching the sidebar over to it.

use gpui::*;

use crate::discord::{self, DirectMessage};
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// Switches the sidebar to the direct-message list, clearing the currently
    /// open conversation. The DM list is fetched the first time and reused on
    /// subsequent opens.
    pub(crate) fn open_direct_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view == View::DirectMessages {
            return;
        }
        self.view = View::DirectMessages;

        // No conversation is open yet; reset the message pane to its empty
        // state so the previously viewed channel's messages don't linger.
        self.selected_channel = None;
        self.messages.clear();
        self.messages_list.reset(0);
        self.image_cache
            .update(cx, |cache, cx| cache.clear(window, cx));
        self.messages_error = None;
        self.send_error = None;
        self.replying_to = None;
        self.older_loading = false;
        self.reached_oldest = false;
        self.message_input.update(cx, |input, cx| {
            input.set_placeholder("Send a message", window, cx);
        });
        cx.notify();

        if !self.dms_loaded && !self.dms_loading {
            self.load_dms(window, cx);
        }
    }

    fn load_dms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dms_error = None;
        self.dms_loading = true;
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.dms_loading = false;
            self.dms_error = Some("No token found. Please log in first.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_dms(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, _window, cx| {
                match result {
                    Ok(dms) => {
                        this.dms = dms;
                        this.dms_loaded = true;
                    }
                    Err(err) => this.dms_error = Some(err),
                }
                this.dms_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// The DM matching the currently open conversation, if the DM view is
    /// active and its channel is one of the loaded conversations.
    pub(crate) fn selected_dm_info(&self) -> Option<&DirectMessage> {
        let id = self.selected_channel?;
        self.dms.iter().find(|dm| dm.id == id)
    }
}
