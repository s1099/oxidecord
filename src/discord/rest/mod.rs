//! The REST calls the app makes against Discord's HTTP API.
//!
//! Each entry point is an `async fn` that runs its request on the shared
//! background Tokio runtime, so it can be awaited straight from a gpui task.

mod channel;
mod guild;
mod message;
mod settings;
mod user;

use std::sync::Arc;

use twilight_http::Client as HttpClient;

use crate::discord::token;
use crate::platform::runtime;

pub use channel::fetch_dms;
pub use guild::{fetch_channels, fetch_guilds};
pub use message::{
    MAX_ATTACHMENT_SIZE, MESSAGE_PAGE_SIZE, delete_message, fetch_messages, send_message,
    toggle_reaction,
};
pub use settings::fetch_guild_folders;
pub use user::{fetch_current_user, fetch_user_profile};

/// Runs a request against the shared client on the runtime, flattening its
/// error to the message the UI shows.
async fn request<T, F>(
    request: impl FnOnce(Arc<HttpClient>) -> F + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
{
    // The client is fetched inside the runtime: building one spawns its
    // ratelimiter task, which panics anywhere else.
    match runtime::run(async move { request(token::client()?).await }).await {
        Ok(result) => result.map_err(|err| err.to_string()),
        Err(err) => Err(err.to_string()),
    }
}
