//! Loading the direct-message list and switching the sidebar over to it.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

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

    /// Keeps the loaded DM list in step with a message that just arrived in a
    /// DM: its conversation moves to the top. A conversation the list doesn't
    /// have yet (someone new, or one the user had closed) needs its name and
    /// avatar, which the message alone doesn't carry, so the list is refetched.
    pub(in crate::screens::home) fn note_dm_activity(
        &mut self,
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        cx: &mut Context<Self>,
    ) {
        if !self.dms_loaded {
            // The first load will fetch the list as it stands.
            return;
        }
        match self.dms.iter().position(|dm| dm.id == channel_id) {
            Some(index) => {
                let mut dm = self.dms.remove(index);
                dm.record_message(message_id.get());
                self.dms.insert(0, dm);
                cx.notify();
            }
            None => self.refresh_dms(cx),
        }
    }

    /// Refetches the DM list without the loading skeleton, keeping the current
    /// list on screen until the new one arrives.
    fn refresh_dms(&mut self, cx: &mut Context<Self>) {
        if self.dms_refreshing {
            return;
        }
        self.dms_refreshing = true;

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_dms().await;
            let _ = this.update(cx, |this, cx| {
                this.dms_refreshing = false;
                match result {
                    Ok(dms) => {
                        this.dms = dms;
                        cx.notify();
                    }
                    Err(err) => eprintln!("refreshing the DM list failed: {err}"),
                }
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
