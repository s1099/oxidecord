//! Reading and writing the user's settings-proto blob.

use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;

use super::request;
use crate::discord::model::{
    PresenceStatus, RawSettingsProto, UserSettings, encode_status_settings, parse_user_settings,
};

/// Fetches the rail's server ordering and the user's chosen status
/// (`GET /users/@me/settings-proto/1`). A user-client endpoint with no
/// twilight helper, so it goes out as a raw request.
pub async fn fetch_user_settings() -> Result<UserSettings, String> {
    request(|client| async move {
        let request =
            RequestBuilder::raw(Method::Get, "users/@me/settings-proto/1".to_owned()).build()?;
        let settings = client
            .request::<RawSettingsProto>(request)
            .await?
            .model()
            .await?;
        parse_user_settings(&settings.settings).map_err(anyhow::Error::msg)
    })
    .await
}

/// Saves the user's status (`PATCH /users/@me/settings-proto/1`), which is
/// what makes it stick: a presence update alone lasts only as long as the
/// session, and Discord's clients reapply the saved one on every connect.
pub async fn save_status(status: PresenceStatus) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Patch, "users/@me/settings-proto/1".to_owned())
            .json(&serde_json::json!({ "settings": encode_status_settings(status) }))
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}
