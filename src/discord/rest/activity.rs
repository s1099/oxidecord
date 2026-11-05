//! Activities: the shelf, starting, joining and leaving an instance, and the
//! OAuth2 calls an activity makes through the client.

use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, GuildMarker, UserMarker},
};

use super::request;
use crate::discord::model::{
    ActivityApp, EntryPoint, RawShelf, convert_shelf, entry_point as find_entry_point,
};
use crate::platform::{http, runtime};

pub async fn fetch_activity_shelf(
    guild_id: Option<Id<GuildMarker>>,
) -> Result<Vec<ActivityApp>, String> {
    request(move |client| async move {
        let path = match guild_id {
            Some(guild_id) => format!("activities/shelf?guild_id={guild_id}"),
            None => "activities/shelf".to_owned(),
        };
        let request = RequestBuilder::raw(Method::Get, path).build()?;
        let shelf = client.request::<RawShelf>(request).await?.model().await?;
        Ok(convert_shelf(shelf))
    })
    .await
}

/// The app's entry point command, if it has one. Like the guild indexes, a 202
/// means Discord is still building it.
pub async fn fetch_entry_point(
    app_id: Id<ApplicationMarker>,
) -> Result<Option<EntryPoint>, String> {
    #[derive(Deserialize)]
    struct RawIndex {
        #[serde(default)]
        application_commands: Vec<serde_json::Value>,
    }

    request(move |client| async move {
        let path = format!("applications/{app_id}/application-command-index");
        for _ in 0..3 {
            let request = RequestBuilder::raw(Method::Get, path.clone()).build()?;
            let response = client.request::<RawIndex>(request).await?;
            if response.status().get() == 202 {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            return Ok(find_entry_point(
                response.model().await?.application_commands,
            ));
        }
        anyhow::bail!("Discord is still preparing this activity")
    })
    .await
}

pub struct ActivityLaunch {
    pub app_id: Id<ApplicationMarker>,
    pub channel_id: Id<ChannelMarker>,
    pub guild_id: Option<Id<GuildMarker>>,
    /// The gateway session's; Discord adds it to the instance's participants,
    /// which is how the client learns it's in.
    pub session_id: String,
    pub nonce: u64,
    /// Without one, the activity is started the older way, by channel.
    pub entry_point: Option<EntryPoint>,
}

/// Starts a new instance as Discord's client does, by running the app's entry
/// point command. The instance arrives as `EMBEDDED_ACTIVITY_UPDATE_V2`.
pub async fn start_activity(launch: ActivityLaunch) -> Result<(), String> {
    let Some(entry) = launch.entry_point else {
        return join_activity(
            launch.app_id,
            launch.channel_id,
            launch.guild_id,
            launch.session_id,
        )
        .await;
    };
    let mut data = serde_json::json!({
        "version": entry.version,
        "id": entry.id,
        "name": entry.name,
        "type": 4,
        "options": [],
        "application_command": entry.raw,
    });
    let mut body = serde_json::json!({
        "type": 2,
        "application_id": launch.app_id,
        "channel_id": launch.channel_id,
        "session_id": launch.session_id,
        "nonce": launch.nonce.to_string(),
        "analytics_location": "activity_shelf",
    });
    if let Some(guild_id) = launch.guild_id {
        data["guild_id"] = guild_id.to_string().into();
        body["guild_id"] = guild_id.to_string().into();
    }
    body["data"] = data;
    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Post, "interactions".to_owned())
            .json(&body)
            .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// Joins the app's running instance in the channel, or starts one if there's none.
pub async fn join_activity(
    app_id: Id<ApplicationMarker>,
    channel_id: Id<ChannelMarker>,
    guild_id: Option<Id<GuildMarker>>,
    session_id: String,
) -> Result<(), String> {
    let mut body = serde_json::json!({ "session_id": session_id });
    if let Some(guild_id) = guild_id {
        body["guild_id"] = guild_id.to_string().into();
    }
    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Post, format!("activities/{channel_id}/{app_id}"))
                .json(&body)
                .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

pub async fn leave_activity(
    app_id: Id<ApplicationMarker>,
    location_id: String,
    instance_id: String,
    session_id: String,
) -> Result<(), String> {
    request(move |client| async move {
        let request = RequestBuilder::raw(
            Method::Post,
            format!("applications/{app_id}/activities/{location_id}/instances/{instance_id}/leave"),
        )
        .json(&serde_json::json!({ "session_id": session_id }))
        .build()?;
        client.request::<EmptyBody>(request).await?;
        Ok(())
    })
    .await
}

/// Lets the activity's page reach the app's backend through Discord's proxy.
pub async fn fetch_proxy_ticket(
    app_id: Id<ApplicationMarker>,
    channel_id: Id<ChannelMarker>,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Ticket {
        ticket: String,
    }

    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Post, format!("applications/{app_id}/proxy-tickets"))
                .json(&serde_json::json!({
                    "use_stateless_ticket": true,
                    "channel_id": channel_id,
                }))
                .build()?;
        Ok(client
            .request::<Ticket>(request)
            .await?
            .model()
            .await?
            .ticket)
    })
    .await
}

/// A user object as Discord serves it, for handing on to an activity. `None`
/// is the signed-in user.
pub async fn fetch_user_object(
    user_id: Option<Id<UserMarker>>,
) -> Result<serde_json::Value, String> {
    request(move |client| async move {
        let path = match user_id {
            Some(id) => format!("users/{id}"),
            None => "users/@me".to_owned(),
        };
        let request = RequestBuilder::raw(Method::Get, path).build()?;
        Ok(client
            .request::<serde_json::Value>(request)
            .await?
            .model()
            .await?)
    })
    .await
}

/// What an activity asked to be authorized for, as `/oauth2/authorize` takes it.
#[derive(Clone, Debug)]
pub struct OAuthRequest {
    pub client_id: Id<ApplicationMarker>,
    pub scopes: Vec<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    /// 0 installs the app to a guild, 1 to the user; see [`fetch_install_type`].
    pub integration_type: u8,
}

impl OAuthRequest {
    fn query(&self) -> String {
        let mut pairs = vec![
            ("client_id", self.client_id.to_string()),
            ("response_type", "code".to_owned()),
            ("scope", self.scopes.join(" ")),
            ("integration_type", self.integration_type.to_string()),
        ];
        let optional = [
            ("state", &self.state),
            ("code_challenge", &self.code_challenge),
            ("code_challenge_method", &self.code_challenge_method),
        ];
        pairs.extend(
            optional
                .into_iter()
                .filter_map(|(key, value)| Some((key, value.clone()?))),
        );
        pairs
            .iter()
            .map(|(key, value)| format!("{key}={}", utf8_percent_encode(value, NON_ALPHANUMERIC)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// How an activity's authorization installs its app, when the activity doesn't
/// say: to the user if the app can be, as Discord's client decides. A guild
/// install asking for `applications.commands` needs a guild picked.
pub async fn fetch_install_type(app_id: Id<ApplicationMarker>) -> Result<u8, String> {
    #[derive(Deserialize)]
    struct RpcApp {
        #[serde(default)]
        integration_types_config: Option<std::collections::HashMap<String, serde_json::Value>>,
    }

    request(move |client| async move {
        let request = RequestBuilder::raw(Method::Get, format!("oauth2/applications/{app_id}/rpc"))
            .build()?;
        let app = client.request::<RpcApp>(request).await?.model().await?;
        let user_install = app
            .integration_types_config
            .is_some_and(|config| config.contains_key("1"));
        Ok(u8::from(user_install))
    })
    .await
}

pub struct AuthorizationPreview {
    /// Already granted these scopes, so Discord's client asks nothing.
    pub authorized: bool,
    pub app_name: String,
    pub app_icon_url: Option<String>,
}

pub async fn preview_authorization(oauth: OAuthRequest) -> Result<AuthorizationPreview, String> {
    #[derive(Deserialize)]
    struct Preview {
        #[serde(default)]
        authorized: bool,
        application: PreviewApp,
    }
    #[derive(Deserialize)]
    struct PreviewApp {
        id: Id<ApplicationMarker>,
        name: String,
        #[serde(default)]
        icon: Option<String>,
    }

    request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Get, format!("oauth2/authorize?{}", oauth.query()))
                .build()?;
        let preview = client.request::<Preview>(request).await?.model().await?;
        let app = preview.application;
        Ok(AuthorizationPreview {
            authorized: preview.authorized,
            app_icon_url: app.icon.map(|hash| {
                format!(
                    "https://cdn.discordapp.com/app-icons/{}/{hash}.webp?size=80",
                    app.id
                )
            }),
            app_name: app.name,
        })
    })
    .await
}

/// Grants the scopes and returns the authorization code Discord redirects with.
pub async fn authorize(
    oauth: OAuthRequest,
    channel_id: Id<ChannelMarker>,
    guild_id: Option<Id<GuildMarker>>,
) -> Result<String, String> {
    let mut body = serde_json::json!({
        "authorize": true,
        "integration_type": oauth.integration_type,
        "location_context": {
            // Discord's client fills an absent one with this placeholder id.
            "guild_id": guild_id.map_or_else(|| "10000".to_owned(), |id| id.to_string()),
            "channel_id": channel_id,
            "channel_type": if guild_id.is_some() { 2 } else { 1 },
        },
    });
    // Discord's consent screen would have the user pick a guild; the one the
    // activity runs in stands in.
    if oauth.integration_type == 0
        && let Some(guild_id) = guild_id
    {
        body["guild_id"] = guild_id.to_string().into();
    }
    let redirect = request(move |client| async move {
        let request =
            RequestBuilder::raw(Method::Post, format!("oauth2/authorize?{}", oauth.query()))
                .json(&body)
                .build()?;
        Ok(client
            .request::<serde_json::Value>(request)
            .await?
            .model()
            .await?)
    })
    .await?;
    authorization_code(&redirect)
}

/// The redirect comes back as `location`, or `url` in older answers, with the
/// code or an error in its query.
fn authorization_code(redirect: &serde_json::Value) -> Result<String, String> {
    let location = redirect
        .get("location")
        .or_else(|| redirect.get("url"))
        .and_then(|value| value.as_str())
        .ok_or("Discord didn't return an authorization")?;
    let url = reqwest::Url::parse(location).map_err(|err| err.to_string())?;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match &*key {
            "code" => return Ok(value.into_owned()),
            "error_description" => error = Some(value.into_owned()),
            "error" if error.is_none() => error = Some(value.into_owned()),
            _ => {}
        }
    }
    Err(error.unwrap_or_else(|| "Discord didn't return an authorization code".to_owned()))
}

/// `GET /oauth2/@me` under the activity's own access token, which is how an
/// activity proves who it's talking to. Sent apart from the shared client, which
/// would sign it with the user's token.
pub async fn fetch_oauth_identity(access_token: String) -> Result<serde_json::Value, String> {
    let result = runtime::run(async move {
        let response = http::client()
            .get("https://discord.com/api/v10/oauth2/@me")
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?;
        let bytes = response.bytes().await?;
        anyhow::Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?)
    })
    .await;
    match result {
        Ok(Ok(identity)) => Ok(identity),
        Ok(Err(err)) => Err(err.to_string()),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_code_or_the_error_from_the_redirect() {
        let ok = serde_json::json!({ "location": "https://example.com/cb?code=abc&state=s" });
        assert_eq!(authorization_code(&ok).unwrap(), "abc");
        let denied = serde_json::json!({ "url": "https://example.com/cb?error=access_denied" });
        assert_eq!(authorization_code(&denied).unwrap_err(), "access_denied");
    }

    #[test]
    fn encodes_scopes_space_separated() {
        let oauth = OAuthRequest {
            client_id: Id::new(1),
            scopes: vec!["identify".into(), "guilds".into()],
            state: Some("a b".into()),
            code_challenge: None,
            code_challenge_method: None,
            integration_type: 1,
        };
        assert_eq!(
            oauth.query(),
            "client_id=1&response_type=code&scope=identify%20guilds&integration_type=1&state=a%20b"
        );
    }
}
