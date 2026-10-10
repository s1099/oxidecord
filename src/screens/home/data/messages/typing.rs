//! Letting the open conversation see the user typing in the composer.

use std::time::{Duration, Instant};

use gpui::*;

use crate::discord;
use crate::screens::home::HomeScreen;

/// How often the indicator is re-sent while the user keeps typing. Discord
/// shows it for ten seconds per trigger; re-sending a little before that keeps
/// it from flickering off between keystrokes.
const TYPING_INTERVAL: Duration = Duration::from_secs(8);

impl HomeScreen {
    /// Called on every change to the composer's text.
    pub(in crate::screens::home) fn on_composer_changed(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected_channel else {
            return;
        };
        // Emptying the composer, by sending or deleting, isn't typing.
        if self.message_input.read(cx).value().trim().is_empty() {
            return;
        }
        if self.typing_sent.is_some_and(|(channel, sent)| {
            channel == channel_id && sent.elapsed() < TYPING_INTERVAL
        }) {
            return;
        }
        self.typing_sent = Some((channel_id, Instant::now()));

        let request = discord::trigger_typing(channel_id);
        // Only cosmetic, so a failure isn't worth surfacing.
        cx.spawn(async move |_, _| {
            let _ = request.await;
        })
        .detach();
    }
}
