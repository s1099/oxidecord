//! Loading the direct-message list and switching the sidebar over to it.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker},
};

use crate::discord::{self, DirectMessage};
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    /// Clears the open conversation. The DM list loads at startup and stays current,
    /// so this fetches only after that failed.
    pub(in crate::screens::home) fn open_direct_messages(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.view == View::DirectMessages {
            return;
        }
        self.view = View::DirectMessages;

        // Reset so the previous channel's messages don't linger.
        self.selected_channel = None;
        self.reset_conversation(None, window, cx);

        if !self.dms_loaded && !self.dms_loading {
            self.load_dms(cx);
        }
    }

    pub(in crate::screens::home) fn load_dms(&mut self, cx: &mut Context<Self>) {
        self.dms_error = None;
        self.dms_loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = discord::fetch_dms().await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(dms) => {
                        this.note_dm_latest(&dms);
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

    /// Moves the conversation to the top. One the list lacks (someone new, or
    /// closed) needs a name and avatar the message doesn't carry, so the list is
    /// refetched.
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

    /// Refetch without the skeleton; the current list stays up meanwhile.
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
                        this.note_dm_latest(&dms);
                        this.dms = dms;
                        cx.notify();
                    }
                    Err(err) => eprintln!("refreshing the DM list failed: {err}"),
                }
            });
        })
        .detach();
    }

    fn note_dm_latest(&mut self, dms: &[DirectMessage]) {
        for dm in dms {
            if let Some(latest) = dm.last_message_id() {
                self.unreads.note_latest(dm.id, latest);
            }
        }
    }

    pub(in crate::screens::home) fn selected_dm_info(&self) -> Option<&DirectMessage> {
        let id = self.selected_channel?;
        self.dms.iter().find(|dm| dm.id == id)
    }
}
