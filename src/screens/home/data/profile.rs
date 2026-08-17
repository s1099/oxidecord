//! Opening the profile popout and loading the user behind it.

use gpui::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord;
use crate::screens::home::{HomeScreen, ProfilePopup};

impl HomeScreen {
    /// Opens the profile card for `user_id`, anchored at `position` (window
    /// coordinates, normally where the avatar was clicked). `name` and
    /// `avatar_url` come from the message that was clicked and fill the card
    /// until the fetch resolves, so it never opens blank.
    ///
    /// Closing is the card's own business: its dismiss layer swallows any press
    /// outside it, so clicking the same avatar again reads as a toggle without
    /// this ever being reached a second time.
    pub(crate) fn open_profile(
        &mut self,
        user_id: Id<UserMarker>,
        name: String,
        avatar_url: Option<String>,
        position: Point<Pixels>,
        window: &mut Window,
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
            self.load_profile(user_id, window, cx);
        }
    }

    pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile_popup.take().is_some() {
            cx.notify();
        }
    }

    fn load_profile(
        &mut self,
        user_id: Id<UserMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(token) = discord::load_token() else {
            if let Some(popup) = &mut self.profile_popup {
                popup.error = Some("No token found. Please log in first.".into());
            }
            cx.notify();
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_user_profile(token, user_id, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, _window, cx| {
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
