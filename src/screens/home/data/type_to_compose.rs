//! Typing with nothing focused writes into the composer, as in Discord: a
//! keystroke no shortcut claimed focuses it and lands there.

use gpui_kit::*;

use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(in crate::screens::home) fn on_type_to_compose(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only from the screen itself. A focused input — the composer, an open
        // edit box, a search — lets its keystrokes bubble up to here too, and
        // takes their text separately.
        if !self.focus_handle.is_focused(window)
            || self.image_viewer.is_some()
            || !self.composer_shown()
        {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        // AltGr arrives as control and alt together, and types characters.
        let alt_gr = modifiers.control && modifiers.alt;
        if modifiers.platform
            || modifiers.function
            || (modifiers.control || modifiers.alt) && !alt_gr
        {
            return;
        }
        let Some(text) = event
            .keystroke
            .key_char
            .clone()
            .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
        else {
            return;
        };

        self.message_input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.insert(text, window, cx);
        });
        // Handled, so the platform doesn't also send the key as text. Nothing
        // would take it — the composer's input handler is only installed when
        // the next frame draws — but a second copy must not be possible.
        cx.stop_propagation();
    }

    /// Whether the open conversation has a composer to type into: a channel
    /// the user can post in, a voice channel with its chat beside it, or a DM.
    fn composer_shown(&self) -> bool {
        match self.view {
            View::DirectMessages => self.selected_dm_info().is_some(),
            View::Guild => self.selected_channel_info().is_some_and(|channel| {
                channel.can_send && (!channel.kind.is_voice() || self.voice_chat_open)
            }),
        }
    }
}
