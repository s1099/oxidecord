//! Storage for the user's Discord token, backed by the OS credential store,
//! and the HTTP client built from it.

use std::sync::{Arc, Mutex, RwLock};

use twilight_http::Client as HttpClient;

const KEYRING_SERVICE: &str = "oxidecord";

const KEYRING_USER: &str = "discord-token";

/// In-process cache so the hot paths don't hit the OS credential store on
/// every request. `None` means not loaded yet.
static TOKEN_CACHE: RwLock<Option<Option<String>>> = RwLock::new(None);

/// One client for every request, so they share its connection pool and
/// ratelimiter. Cleared when the token changes.
static CLIENT: Mutex<Option<Arc<HttpClient>>> = Mutex::new(None);

fn keyring_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
}

pub fn load_token() -> Option<String> {
    if let Some(cached) = TOKEN_CACHE.read().ok()?.as_ref() {
        return cached.clone();
    }

    let token = match keyring_entry().and_then(|entry| entry.get_password()) {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            eprintln!("failed to read token from the credential store: {err}");
            None
        }
    };

    if let Ok(mut cache) = TOKEN_CACHE.write() {
        *cache = Some(token.clone());
    }
    token
}

pub fn save_token(token: &str) -> keyring::Result<()> {
    keyring_entry()?.set_password(token)?;
    if let Ok(mut cache) = TOKEN_CACHE.write() {
        *cache = Some(Some(token.to_owned()));
    }
    if let Ok(mut client) = CLIENT.lock() {
        *client = None;
    }
    Ok(())
}

/// The shared client for the stored token. Must be called on the Tokio
/// runtime, since building the client spawns its ratelimiter there.
pub(super) fn client() -> anyhow::Result<Arc<HttpClient>> {
    let mut client = CLIENT
        .lock()
        .map_err(|_| anyhow::anyhow!("the HTTP client lock was poisoned"))?;
    if let Some(client) = client.as_ref() {
        return Ok(client.clone());
    }
    let token =
        load_token().ok_or_else(|| anyhow::anyhow!("No token found. Please log in first."))?;
    Ok(client.insert(Arc::new(HttpClient::new(token))).clone())
}
