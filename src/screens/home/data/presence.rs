//! The user's own online status, picked from the account panel.

use gpui::*;

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// Switches the user's status: on the gateway at once, so everyone sees
    /// it, and in settings-proto, so it sticks across restarts and clients.
    pub(in crate::screens::home) fn set_presence_status(
        &mut self,
        status: discord::PresenceStatus,
        cx: &mut Context<Self>,
    ) {
        if self.presence_status == Some(status) {
            return;
        }
        self.apply_presence_status(status);
        cx.notify();

        cx.spawn(async move |_, _| {
            if let Err(err) = discord::save_status(status).await {
                eprintln!("failed to save status: {err}");
            }
        })
        .detach();
    }

    /// Takes `status` as the user's, telling the gateway. Commands sent before
    /// the session has identified wait for it, so this is safe at any point.
    pub(in crate::screens::home) fn apply_presence_status(
        &mut self,
        status: discord::PresenceStatus,
    ) {
        self.presence_status = Some(status);
        if let Some(gateway) = &self.gateway {
            gateway.update_presence(status);
        }
    }
}
