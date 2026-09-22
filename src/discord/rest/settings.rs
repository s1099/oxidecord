//! Reading the user's settings-proto blob.

use twilight_http::request::{Method, RequestBuilder};

use super::request;
use crate::discord::model::{GuildFolders, RawSettingsProto, parse_guild_folders};

/// Fetches the rail's server ordering (`GET /users/@me/settings-proto/1`,
/// `PreloadedUserSettings.guild_folders`). A user-client endpoint with no
/// twilight helper, so it goes out as a raw request.
pub async fn fetch_guild_folders() -> Result<GuildFolders, String> {
    request(|client| async move {
        let request =
            RequestBuilder::raw(Method::Get, "users/@me/settings-proto/1".to_owned()).build()?;
        let settings = client
            .request::<RawSettingsProto>(request)
            .await?
            .model()
            .await?;
        parse_guild_folders(&settings.settings).map_err(anyhow::Error::msg)
    })
    .await
}
