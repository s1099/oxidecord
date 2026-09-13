//! Reading the user's settings-proto blob.

use twilight_http::Client as HttpClient;
use twilight_http::request::{Method, RequestBuilder};

use crate::platform::runtime;

use crate::discord::model::{GuildFolders, RawSettingsProto, parse_guild_folders};

/// Fetches the rail's server ordering (`GET /users/@me/settings-proto/1`,
/// `PreloadedUserSettings.guild_folders`). A user-client endpoint with no
/// twilight helper, so it goes out as a raw request.
pub fn fetch_guild_folders(
    token: String,
    on_done: impl FnOnce(Result<GuildFolders, String>) + Send + 'static,
) {
    runtime::handle().spawn(async move {
        let result = async {
            let request = RequestBuilder::raw(Method::Get, "users/@me/settings-proto/1".to_owned())
                .build()
                .map_err(|err| err.to_string())?;

            let settings = HttpClient::new(token)
                .request::<RawSettingsProto>(request)
                .await
                .map_err(|err| err.to_string())?
                .model()
                .await
                .map_err(|err| err.to_string())?;

            parse_guild_folders(&settings.settings)
        }
        .await;

        on_done(result);
    });
}
