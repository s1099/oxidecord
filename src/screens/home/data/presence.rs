//! Online statuses: the user's own, picked from the account panel, and
//! everyone else's as the gateway reports them.

use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord;
use crate::screens::home::HomeScreen;

impl HomeScreen {
    /// On the gateway at once, and in settings-proto so it persists across restarts
    /// and clients.
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

    /// Commands sent before the session has identified wait for it.
    pub(in crate::screens::home) fn apply_presence_status(
        &mut self,
        status: discord::PresenceStatus,
    ) {
        self.presence_status = Some(status);
        if let Some(gateway) = &self.gateway {
            gateway.update_presence(status);
        }
    }

    /// The gateway never reports the user's own presence, so theirs is the picked
    /// one; anyone it hasn't mentioned is offline.
    pub(in crate::screens::home) fn user_status(
        &self,
        user_id: Id<UserMarker>,
    ) -> discord::PresenceStatus {
        if Some(user_id) == self.self_user_id {
            return self.presence_status.unwrap_or_default();
        }
        self.user_presences
            .get(&user_id)
            .copied()
            .unwrap_or(discord::PresenceStatus::Invisible)
    }
}
