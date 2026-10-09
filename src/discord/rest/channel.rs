//! Fetching the user's direct-message conversations, and ringing them.

use twilight_http::request::{Method, Request, RequestBuilder};
use twilight_http::response::marker::{EmptyBody, ListBody};
use twilight_http::routing::Route;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::request;
use crate::discord::model::{DirectMessage, convert_dms};

/// Fetches the current user's open DM and group-DM conversations, ordered
/// most-recently-active first.
///
/// twilight has no typed helper for `GET /users/@me/channels`, so this issues
/// the route through the client's low-level request path.
pub async fn fetch_dms() -> Result<Vec<DirectMessage>, String> {
    request(|client| async move {
        let request = Request::from_route(&Route::GetUserPrivateChannels);
        let channels = client
            .request::<ListBody<twilight_model::channel::Channel>>(request)
            .await?
            .models()
            .await?;
        Ok(convert_dms(channels))
    })
    .await
}

/// Rings everyone in a DM's call (`POST /channels/{id}/call/ring`).
///
/// Joining a DM's voice channel only starts the call; nobody is rung until a
/// client asks, as Discord's own does right after joining. A user-client
/// endpoint with no twilight helper. A `null` recipient list rings them all.
pub async fn ring_call(channel_id: Id<ChannelMarker>) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, format!("channels/{channel_id}/call/ring"))
            .json(&serde_json::json!({ "recipients": null }))
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}
