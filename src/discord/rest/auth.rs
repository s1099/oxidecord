//! Ending the session.

use twilight_http::Client as HttpClient;
use twilight_http::request::{Method, RequestBuilder};
use twilight_http::response::marker::EmptyBody;

use crate::discord::token;
use crate::platform::runtime;

/// Logs out: forgets the stored token at once so the app returns to login, then revokes
/// the session in the background (`POST /auth/logout`), as Discord's own clients do.
/// Fire-and-forget: being offline, or a token already expired, shouldn't keep the user
/// signed in here.
pub fn log_out() {
    let Some(token) = token::load_token() else {
        return;
    };
    token::forget_token(&token);

    runtime::handle().spawn(async move {
        // A client of its own, since the shared one went with the token. Built
        // in here because building one spawns its ratelimiter on the runtime.
        let client = HttpClient::new(token);
        let result = async {
            let request = RequestBuilder::raw(Method::Post, "auth/logout".to_owned())
                .json(&serde_json::json!({ "provider": null, "voip_provider": null }))
                .build()?;
            client.request::<EmptyBody>(request).await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = result {
            eprintln!("failed to revoke the session on logout: {err}");
        }
    });
}
