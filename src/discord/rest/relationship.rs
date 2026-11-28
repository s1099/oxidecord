//! Friends, friend requests and blocks. twilight has no routes for these, so each is a
//! raw request; Discord answers each change with a `RELATIONSHIP_*` dispatch, which is what
//! updates the app.

use twilight_http::api_error::ApiError;
use twilight_http::error::ErrorType;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{Id, marker::UserMarker};

use super::request;

/// No user goes by that name.
const UNKNOWN_USERNAME: u64 = 80004;

/// What Discord's client says when a username finds nobody.
const UNKNOWN_USERNAME_MESSAGE: &str =
    "Hm, didn't work. Double check that the username is correct.";

/// Sends a friend request to whoever goes by `username`.
pub async fn send_friend_request(username: String) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, "users/@me/relationships".to_owned())
            .json(&serde_json::json!({ "username": username, "discriminator": null }))
            .build()?;
        client
            .request::<EmptyBody>(request)
            .await
            .map_err(friendly_error)?;
        Ok(())
    })
    .await
}

/// Accepts their request, or sends one by id.
pub async fn add_friend(user_id: Id<UserMarker>) -> Result<(), String> {
    put_relationship(user_id, serde_json::json!({})).await
}

pub async fn block_user(user_id: Id<UserMarker>) -> Result<(), String> {
    put_relationship(user_id, serde_json::json!({ "type": 2 })).await
}

/// Unfriends, cancels or ignores a request, or unblocks, whichever the relationship is.
pub async fn remove_relationship(user_id: Id<UserMarker>) -> Result<(), String> {
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Delete, format!("users/@me/relationships/{user_id}"))
                .build()?;
        client
            .request::<EmptyBody>(request)
            .await
            .map_err(friendly_error)?;
        Ok(())
    })
    .await
}

/// `None` clears the nickname.
pub async fn set_friend_nickname(
    user_id: Id<UserMarker>,
    nickname: Option<String>,
) -> Result<(), String> {
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Patch, format!("users/@me/relationships/{user_id}"))
                .json(&serde_json::json!({ "nickname": nickname }))
                .build()?;
        client
            .request::<EmptyBody>(request)
            .await
            .map_err(friendly_error)?;
        Ok(())
    })
    .await
}

async fn put_relationship(user_id: Id<UserMarker>, body: serde_json::Value) -> Result<(), String> {
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Put, format!("users/@me/relationships/{user_id}"))
                .json(&body)
                .build()?;
        client
            .request::<EmptyBody>(request)
            .await
            .map_err(friendly_error)?;
        Ok(())
    })
    .await
}

/// Discord's own message ("You're already friends with that user!") rather than the status
/// line and raw body twilight's error shows. Other errors pass through so a refused token
/// is still recognised.
fn friendly_error(err: twilight_http::Error) -> anyhow::Error {
    if let ErrorType::Response {
        error: ApiError::General(error),
        ..
    } = err.kind()
    {
        if error.code == UNKNOWN_USERNAME {
            return anyhow::anyhow!(UNKNOWN_USERNAME_MESSAGE);
        }
        if !error.message.is_empty() && error.code != 0 {
            return anyhow::anyhow!(error.message.clone());
        }
    }
    err.into()
}
