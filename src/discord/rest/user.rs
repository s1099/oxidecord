//! Fetching the signed-in user and other users' profiles.

use twilight_http::request::{Method, RequestBuilder};
use twilight_model::id::{Id, marker::UserMarker};

use super::request;
use crate::discord::model::{
    CurrentUser, RawProfile, UserProfile, convert_current_user, convert_user_profile,
};

/// Fetches the signed-in user (`GET /users/@me`).
pub async fn fetch_current_user() -> Result<CurrentUser, String> {
    request(|client| async move {
        let user = client.current_user().await?.model().await?;
        Ok(convert_current_user(user))
    })
    .await
}

/// Fetches another user's profile, for the popout opened from their avatar.
///
/// Prefers `GET /users/{id}/profile` — the endpoint the Discord client itself
/// uses, and the only one carrying the bio and the global (non-guild) banner.
/// It has no twilight helper, so it goes out as a raw request. Tokens that
/// can't reach it (a bot token, for instance) fall back to the plain user
/// object, which covers everything but the bio and pronouns.
pub async fn fetch_user_profile(user_id: Id<UserMarker>) -> Result<UserProfile, String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(
            Method::Get,
            format!("users/{user_id}/profile?with_mutual_guilds=false"),
        )
        .build()?;

        let profile = async {
            let response = client.request::<RawProfile>(request).await.ok()?;
            Some(response.model().await.ok()?.into_profile())
        }
        .await;
        if let Some(profile) = profile {
            return Ok(profile);
        }

        let user = client.user(user_id).await?.model().await?;
        Ok(convert_user_profile(user))
    })
    .await
}
