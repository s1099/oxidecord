use std::sync::OnceLock;

use serde::Deserialize;
use tokio::runtime::Handle;
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::GuildMarker, Id};

static RUNTIME: OnceLock<Handle> = OnceLock::new();

/// Returns a handle to a lazily-started background Tokio runtime.
///
/// twilight-http needs to run inside a Tokio context, but gpui has its own
/// executor, so we keep a dedicated runtime alive on its own thread.
pub fn runtime_handle() -> &'static Handle {
    RUNTIME.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            tx.send(rt.handle().clone())
                .expect("failed to send tokio runtime handle");
            rt.block_on(std::future::pending::<()>());
        });
        rx.recv().expect("failed to receive tokio runtime handle")
    })
}

#[derive(Deserialize)]
struct AuthFile {
    #[serde(rename = "userToken")]
    user_token: String,
}

/// Reads the user's Discord token from `auth.json` in the working directory.
pub fn load_token() -> Option<String> {
    let content = std::fs::read_to_string("auth.json").ok()?;
    serde_json::from_str::<AuthFile>(&content)
        .ok()
        .map(|auth| auth.user_token)
}

#[derive(Clone)]
pub struct Guild {
    pub id: Id<GuildMarker>,
    pub name: String,
    pub icon_url: Option<String>,
}

/// Fetches the current user's guilds and invokes `on_done` with the result.
///
/// Runs on the background Tokio runtime; `on_done` is called from that
/// runtime's thread, not the gpui foreground thread.
pub fn fetch_guilds(
    token: String,
    on_done: impl FnOnce(Result<Vec<Guild>, String>) + Send + 'static,
) {
    runtime_handle().spawn(async move {
        let result = async {
            let client = HttpClient::new(token);
            let response = client
                .current_user_guilds()
                .await
                .map_err(|err| err.to_string())?;
            let guilds = response.models().await.map_err(|err| err.to_string())?;

            Ok(guilds
                .into_iter()
                .map(|guild| {
                    let icon_url = guild.icon.map(|hash| {
                        format!("https://cdn.discordapp.com/icons/{}/{}.webp?size=100&quality=lossless", guild.id, hash)
                    });
                    Guild {
                        id: guild.id,
                        name: guild.name,
                        icon_url,
                    }
                })
                .collect::<Vec<_>>())
        }
        .await;

        on_done(result);
    });
}
