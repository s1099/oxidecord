//! Fetching the user's direct-message conversations, and ringing them.

use twilight_http::request::{Method, Request, RequestBuilder};
use twilight_http::response::marker::{EmptyBody, ListBody};
use twilight_http::routing::Route;
use twilight_model::id::{Id, marker::ChannelMarker};

use super::request;
use crate::discord::model::{DirectMessage, convert_dms};

/// Fetches the user's open DMs and group DMs, most recently active first
/// (`GET /users/@me/channels`; no twilight helper, so a raw request).
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

/// Rings everyone in a DM's call (`POST /channels/{id}/call/ring`). Joining only starts
/// the call; nobody is rung until a client asks. A `null` recipient list rings them all.
///
/// Being able to call someone doesn't mean being allowed to ring them, so it first asks
/// `GET /channels/{id}/call`, as Discord's client does. A user who isn't `ringable` would
/// get a 403 (50013); the call still goes ahead without the ring.
pub async fn ring_call(channel_id: Id<ChannelMarker>) -> Result<(), String> {
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Get, format!("channels/{channel_id}/call")).build()?;
        let eligibility = client
            .request::<CallEligibility>(request)
            .await?
            .model()
            .await?;
        if !eligibility.ringable {
            return Ok(());
        }

        let request = RequestBuilder::raw(Method::Post, format!("channels/{channel_id}/call/ring"))
            .json(&serde_json::json!({ "recipients": null }))
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// `GET /channels/{id}/call`: whether the user may ring the DM's other members.
#[derive(serde::Deserialize)]
struct CallEligibility {
    ringable: bool,
}

/// Stops a DM's call ringing the current user
/// (`POST /channels/{id}/call/stop-ringing`): declining it, as far as anyone
/// can tell. The call itself carries on for whoever is in it.
pub async fn stop_ringing(channel_id: Id<ChannelMarker>) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(
            Method::Post,
            format!("channels/{channel_id}/call/stop-ringing"),
        )
        .json(&serde_json::json!({ "recipients": null }))
        .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}
