//! Fetching the user's direct-message conversations.

use twilight_http::request::Request;
use twilight_http::response::marker::ListBody;
use twilight_http::routing::Route;

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
