//! Fetching the user's direct-message conversations.

use twilight_http::Client as HttpClient;
use twilight_http::request::Request;
use twilight_http::response::marker::ListBody;
use twilight_http::routing::Route;

use crate::platform::runtime;

use crate::discord::model::{DirectMessage, convert_dms};

/// Fetches the current user's open DM and group-DM conversations, ordered
/// most-recently-active first.
///
/// twilight has no typed helper for `GET /users/@me/channels`, so this issues
/// the route through the client's low-level request path.
pub fn fetch_dms(
    token: String,
    on_done: impl FnOnce(Result<Vec<DirectMessage>, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let request = Request::from_route(&Route::GetUserPrivateChannels);
            let response = client
                .request::<ListBody<twilight_model::channel::Channel>>(request)
                .await
                .map_err(|err| err.to_string())?;
            let channels = response.models().await.map_err(|err| err.to_string())?;

            Ok(convert_dms(channels))
        }
        .await;

        on_done(result);
    });
}
