//! Typing indicators: letting the open conversation see the user typing in
//! the composer, and showing who else is typing there.

use std::time::{Duration, Instant};

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, UserMarker},
};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::state::Typist;

/// How often the indicator is re-sent while the user keeps typing. Discord
/// shows it for ten seconds per trigger; re-sending a little before that keeps
/// it from flickering off between keystrokes.
const TYPING_INTERVAL: Duration = Duration::from_secs(8);

/// How long someone else's indicator lasts without being renewed.
const TYPING_TIMEOUT: Duration = Duration::from_secs(10);

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

    /// Someone started, or is still, typing in `channel_id`.
    pub(in crate::screens::home) fn handle_typing(
        &mut self,
        channel_id: Id<ChannelMarker>,
        user_id: Id<UserMarker>,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // The user's own typing echoes back; the composer already shows it.
        if Some(user_id) == self.self_user_id {
            return;
        }
        let until = Instant::now() + TYPING_TIMEOUT;
        let typists = self.typists.entry(channel_id).or_default();
        match typists.iter_mut().find(|typist| typist.user_id == user_id) {
            Some(typist) => {
                typist.until = until;
                typist.name = name.or(typist.name.take());
            }
            None => typists.push(Typist {
                user_id,
                name,
                until,
            }),
        }
        if self.selected_channel == Some(channel_id) {
            cx.notify();
        }

        // Nothing else says when an indicator lapses, so each one wakes the
        // screen when it would. A renewal in between makes this a no-op.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| this.prune_typists(channel_id, cx));
        })
        .detach();
    }

    /// Their message arrived, which ends their typing early.
    pub(in crate::screens::home) fn stop_typing(
        &mut self,
        channel_id: Id<ChannelMarker>,
        user_id: Id<UserMarker>,
        cx: &mut Context<Self>,
    ) {
        if let Some(typists) = self.typists.get_mut(&channel_id) {
            typists.retain(|typist| typist.user_id != user_id);
            if typists.is_empty() {
                self.typists.remove(&channel_id);
            }
            cx.notify();
        }
    }

    fn prune_typists(&mut self, channel_id: Id<ChannelMarker>, cx: &mut Context<Self>) {
        let Some(typists) = self.typists.get_mut(&channel_id) else {
            return;
        };
        let now = Instant::now();
        let before = typists.len();
        typists.retain(|typist| typist.until > now);
        let changed = typists.len() != before;
        if typists.is_empty() {
            self.typists.remove(&channel_id);
        }
        if changed && self.selected_channel == Some(channel_id) {
            cx.notify();
        }
    }
}
