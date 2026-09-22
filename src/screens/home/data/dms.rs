//! Loading the direct-message list and switching the sidebar over to it.

use gpui::*;

use crate::discord::{self, DirectMessage};
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// Switches the sidebar to the direct-message list, clearing the currently
    /// open conversation. The DM list is fetched the first time and reused on
    /// subsequent opens.
    pub(in crate::screens::home) fn open_direct_messages(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::DirectMessages {
            return;
        }
        self.view = View::DirectMessages;

        // No conversation is open yet; reset the message pane to its empty
        // state so the previously viewed channel's messages don't linger.
        self.selected_channel = None;
        self.reset_conversation(None, window, cx);

        if !self.dms_loaded && !self.dms_loading {
            self.load_dms(cx);
        }
    }

    fn load_dms(&mut self, cx: &mut Context<Self>) {
        self.dms_error = None;
        self.dms_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_dms().await;
            let _ = this.update(cx, |this, cx| {
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
    pub(in crate::screens::home) fn selected_dm_info(&self) -> Option<&DirectMessage> {
        let id = self.selected_channel?;
        self.dms.iter().find(|dm| dm.id == id)
    }
}
