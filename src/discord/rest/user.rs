//! Fetching the signed-in user and other users' profiles.

use serde::Deserialize;
use twilight_http::error::ErrorType;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{
    Id,
    marker::{GuildMarker, UserMarker},
};

use super::request;
use crate::discord::model::{
    CurrentUser, GameInfo, RawGame, RawProfile, UserProfile, convert_current_user,
    convert_user_profile,
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

#[derive(Deserialize)]
struct RawNote {
    #[serde(default)]
    note: String,
}

/// The private note the user keeps on `user_id`; empty when there is none,
/// which Discord answers with a 404.
pub async fn fetch_user_note(user_id: Id<UserMarker>) -> Result<String, String> {
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Get, format!("users/@me/notes/{user_id}")).build()?;
        match client.request::<RawNote>(request).await {
            Ok(response) => Ok(response.model().await?.note),
            Err(err) if matches!(err.kind(), ErrorType::Response { status, .. } if status.get() == 404) => {
                Ok(String::new())
            }
            Err(err) => Err(err.into()),
        }
    })
    .await
}

pub async fn save_user_note(user_id: Id<UserMarker>, note: String) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Put, format!("users/@me/notes/{user_id}"))
            .json(&serde_json::json!({ "note": note }))
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// Names and art for the games on a profile's board, which carries only their
/// ids. Discord's client asks for twenty at a time.
pub async fn fetch_games(ids: Vec<u64>) -> Result<Vec<GameInfo>, String> {
    request(move |client| async move {
        let mut games = Vec::new();
        for chunk in ids.chunks(20) {
            let query: Vec<String> = chunk.iter().map(|id| format!("game_ids={id}")).collect();
            let request =
                RequestBuilder::raw(Method::Get, format!("games?{}", query.join("&"))).build()?;
            let raw = client
                .request::<Vec<RawGame>>(request)
                .await?
                .model()
                .await?;
            games.extend(raw.into_iter().map(RawGame::into_game));
        }
        Ok(games)
    })
    .await
}
