//! Fetching the signed-in user and other users' profiles.

use twilight_http::request::{Method, RequestBuilder};
use twilight_model::id::{
    Id,
    marker::{GuildMarker, UserMarker},
};

use super::request;
use crate::discord::model::{
    CurrentUser, RawProfile, UserProfile, convert_current_user, convert_user_profile,
};

pub async fn fetch_current_user() -> Result<CurrentUser, String> {
    request(|client| async move {
        let user = client.current_user().await?.model().await?;
        Ok(convert_current_user(user))
    })
    .await
}

/// Fetches another user's profile for the avatar popout. Prefers `GET /users/{id}/profile`,
/// the only endpoint with the bio, badges and mutuals; no twilight helper, so a raw request.
/// With `guild_id` it also carries their server profile and roles there. Tokens that can't
/// reach it (a bot's) fall back to the plain user object, which lacks all of that.
pub async fn fetch_user_profile(
    user_id: Id<UserMarker>,
    guild_id: Option<Id<GuildMarker>>,
) -> Result<UserProfile, String> {
    request(move |client| async move {
        let mut path = format!(
            "users/{user_id}/profile?type=popout&with_mutual_guilds=true\
             &with_mutual_friends=true&with_mutual_friends_count=true"
        );
        if let Some(guild_id) = guild_id {
            path.push_str(&format!("&guild_id={guild_id}"));
        }
        let request = RequestBuilder::raw(Method::Get, path).build()?;

        let profile = async {
            let response = client.request::<RawProfile>(request).await.ok()?;
            Some(response.model().await.ok()?.into_profile(guild_id))
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
