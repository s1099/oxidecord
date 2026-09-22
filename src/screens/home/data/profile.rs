//! Opening the profile popout and loading the user behind it.

use gpui::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord;
use crate::screens::home::{HomeScreen, ProfilePopup};

impl HomeScreen {
    /// Opens the profile card for `user_id`, anchored at `position`. `name` and
    /// `avatar_url` come from the message that was clicked, so the card never
    /// opens blank while the fetch is in flight.
    ///
    /// Closing is the card's own business: its dismiss layer swallows any press
    /// outside it, so a second click on the same avatar never reaches this.
    pub(in crate::screens::home) fn open_profile(
        &mut self,
        user_id: Id<UserMarker>,
        name: String,
        avatar_url: Option<String>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let cached = self.profile_cache.get(&user_id).cloned();
        self.profile_popup = Some(ProfilePopup {
            user_id,
            position,
            name,
            avatar_url,
            profile: cached.clone(),
            error: None,
        });
        cx.notify();

        if cached.is_none() {
            self.load_profile(user_id, cx);
        }
    }

    pub(in crate::screens::home) fn close_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile_popup.take().is_some() {
            cx.notify();
        }
    }

    fn load_profile(&mut self, user_id: Id<UserMarker>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_user_profile(user_id).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(profile) => {
                        this.profile_cache.insert(user_id, profile.clone());
                        // The card may have been closed, or reopened for
                        // somebody else, while the request was in flight.
                        if let Some(popup) = &mut this.profile_popup
                            && popup.user_id == user_id
                        {
                            popup.profile = Some(profile);
                        }
                    }
                    Err(err) => {
                        if let Some(popup) = &mut this.profile_popup
                            && popup.user_id == user_id
                        {
                            popup.error = Some(err);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
