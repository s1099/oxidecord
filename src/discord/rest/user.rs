//! Fetching the signed-in user and other users' profiles.

use twilight_http::Client as HttpClient;
use twilight_http::request::{Method, RequestBuilder};
use twilight_model::id::{Id, marker::UserMarker};

use crate::platform::runtime;

use crate::discord::model::{
    CurrentUser, RawProfile, UserProfile, convert_current_user, convert_user_profile,
};

/// Fetches the signed-in user (`GET /users/@me`).
pub fn fetch_current_user(
    token: String,
    on_done: impl FnOnce(Result<CurrentUser, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client.current_user().await.map_err(|err| err.to_string())?;
            let user = response.model().await.map_err(|err| err.to_string())?;
            Ok(convert_current_user(user))
        }
        .await;

        on_done(result);
    });
}

/// Fetches another user's profile, for the popout opened from their avatar.
///
/// Prefers `GET /users/{id}/profile` — the endpoint the Discord client itself
/// uses, and the only one carrying the bio and the global (non-guild) banner.
/// It has no twilight helper, so it goes out as a raw request. Tokens that
/// can't reach it (a bot token, for instance) fall back to the plain user
/// object, which covers everything but the bio and pronouns.
pub fn fetch_user_profile(
    token: String,
    user_id: Id<UserMarker>,
    on_done: impl FnOnce(Result<UserProfile, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);

            let request = RequestBuilder::raw(
                Method::Get,
                format!("users/{user_id}/profile?with_mutual_guilds=false"),
            )
            .build()
            .map_err(|err| err.to_string())?;

            let profile = async {
                let response = client.request::<RawProfile>(request).await.ok()?;
                Some(response.model().await.ok()?.into_profile())
            }
            .await;
            if let Some(profile) = profile {
                return Ok(profile);
            }

            let user = client
                .user(user_id)
                .await
                .map_err(|err| err.to_string())?
                .model()
                .await
                .map_err(|err| err.to_string())?;
            Ok(convert_user_profile(user))
        }
        .await;

        on_done(result);
    });
}
