//! The text chat every voice channel carries, opened beside its stage.
//!
//! It isn't a conversation of its own: while it's open, the voice channel is
//! the selected channel and the ordinary message plumbing serves it.

use gpui_kit::*;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::screens::home::HomeScreen;

impl HomeScreen {
    pub(in crate::screens::home) fn show_voice_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_channel == Some(channel_id) {
            return;
        }
        self.selected_channel = Some(channel_id);
        if self.voice_chat_open {
            self.load_messages(window, cx);
        } else {
            // The stage doesn't show the conversation that was open, so it's
            // put away rather than left playing video or holding images.
            self.reset_conversation(None, window, cx);
        }
    }

    pub(in crate::screens::home) fn open_voice_chat(
        &mut self,
        channel_id: Id<ChannelMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.voice_chat_open && self.selected_channel == Some(channel_id) {
            return;
        }
        self.voice_chat_open = true;
        self.selected_channel = Some(channel_id);
        self.load_messages(window, cx);
        cx.notify();
    }

    pub(in crate::screens::home) fn close_voice_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.voice_chat_open = false;
        // Cached on the way out, so opening it again is instant.
        self.reset_conversation(None, window, cx);
    }
}
