//! The signed-in accounts: each token in the OS credential store under its user id, and an
//! index of who they belong to (`accounts.json`), since credential stores can't be listed.
//! One account is active at a time; the HTTP client is built from its token.

use std::fs;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use twilight_http::Client as HttpClient;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::CurrentUser;
use crate::platform::prefs;

const KEYRING_SERVICE: &str = "oxidecord";

/// Where the single token lived before multiple accounts; moved on first load.
const LEGACY_KEYRING_USER: &str = "discord-token";

const INDEX_FILE: &str = "accounts.json";

/// What the switcher shows for an account. Empty names until its profile is first fetched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Id<UserMarker>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

impl Account {
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.id.to_string()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    active: Option<Id<UserMarker>>,
    /// Most recently used first.
    #[serde(default)]
    accounts: Vec<Account>,
}

#[derive(Default)]
struct State {
    /// `None` until read from disk.
    index: Option<Index>,
    /// The active account's token. Spares the credential store a hit per request; outer
    /// `None` means not loaded yet.
    token: Option<Option<String>>,
    /// Shared so requests use one connection pool and ratelimiter. Cleared when the
    /// active token changes.
    client: Option<Arc<HttpClient>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl State {
    fn index(&mut self) -> &mut Index {
        self.index.get_or_insert_with(read_index)
    }

    fn active_token(&mut self) -> Option<String> {
        if let Some(token) = &self.token {
            return token.clone();
        }
        let token = self.index().active.and_then(read_stored_token);
        self.token = Some(token.clone());
        token
    }

    fn reset_session(&mut self) {
        self.token = None;
        self.client = None;
    }
}

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(state().get_or_insert_with(State::default))
}

fn keyring_entry(id: Id<UserMarker>) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{LEGACY_KEYRING_USER}:{id}"))
}

fn read_stored_token(id: Id<UserMarker>) -> Option<String> {
    match keyring_entry(id).and_then(|entry| entry.get_password()) {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            eprintln!("failed to read token from the credential store: {err}");
            None
        }
    }
}

fn delete_stored_token(id: Id<UserMarker>) {
    match keyring_entry(id).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(err) => eprintln!("failed to remove token from the credential store: {err}"),
    }
}

/// A user token's first segment is the user's id in base64, so the account is known
/// before any request.
fn user_id_of(token: &str) -> Option<Id<UserMarker>> {
    let head = token.split('.').next()?.trim_end_matches('=');
    let bytes = STANDARD_NO_PAD
        .decode(head)
        .or_else(|_| URL_SAFE_NO_PAD.decode(head))
        .ok()?;
    Id::new_checked(std::str::from_utf8(&bytes).ok()?.parse().ok()?)
}

fn read_index() -> Index {
    let index = prefs::config_file(INDEX_FILE)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default();
    migrate_legacy(index)
}

fn write_index(index: &Index) {
    let Some(path) = prefs::config_file(INDEX_FILE) else {
        return;
    };
    let Ok(contents) = serde_json::to_string_pretty(index) else {
        return;
    };
    if let Some(dir) = path.parent()
        && fs::create_dir_all(dir).is_err()
    {
        return;
    }
    if let Err(err) = fs::write(path, contents) {
        eprintln!("failed to save the account list: {err}");
    }
}

/// Moves a token saved before multiple accounts into its per-account entry.
fn migrate_legacy(mut index: Index) -> Index {
    let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, LEGACY_KEYRING_USER) else {
        return index;
    };
    let Ok(token) = entry.get_password() else {
        return index;
    };
    let Some(id) = user_id_of(&token) else {
        return index;
    };
    if let Err(err) = keyring_entry(id).and_then(|entry| entry.set_password(&token)) {
        eprintln!("failed to move the saved token: {err}");
        return index;
    }
    _ = entry.delete_credential();
    if !index.accounts.iter().any(|account| account.id == id) {
        index.accounts.insert(0, blank_account(id));
    }
    index.active = Some(id);
    write_index(&index);
    index
}

fn blank_account(id: Id<UserMarker>) -> Account {
    Account {
        id,
        name: String::new(),
        username: String::new(),
        avatar_url: None,
    }
}

/// The active account's token.
pub fn load_token() -> Option<String> {
    with_state(State::active_token)
}

/// Every saved account, most recently used first.
pub fn accounts() -> Vec<Account> {
    with_state(|state| state.index().accounts.clone())
}

pub fn active_account() -> Option<Id<UserMarker>> {
    with_state(|state| state.index().active)
}

/// Saves `token` as an account (replacing that account's old one) and makes it active.
pub fn save_token(token: &str) -> anyhow::Result<()> {
    let id = user_id_of(token).ok_or_else(|| anyhow::anyhow!("unrecognised token format"))?;
    keyring_entry(id)?.set_password(token)?;
    with_state(|state| {
        let index = state.index();
        let account = match index.accounts.iter().position(|account| account.id == id) {
            Some(ix) => index.accounts.remove(ix),
            None => blank_account(id),
        };
        index.accounts.insert(0, account);
        index.active = Some(id);
        write_index(index);
        state.reset_session();
        state.token = Some(Some(token.to_owned()));
    });
    Ok(())
}

/// Makes a saved account active. Fails, and drops it from the list, if its token is gone.
pub fn switch_account(id: Id<UserMarker>) -> anyhow::Result<()> {
    let Some(token) = read_stored_token(id) else {
        with_state(|state| {
            let index = state.index();
            index.accounts.retain(|account| account.id != id);
            write_index(index);
        });
        anyhow::bail!("that account's sign-in is no longer saved");
    };
    with_state(|state| {
        let index = state.index();
        let Some(ix) = index.accounts.iter().position(|account| account.id == id) else {
            return;
        };
        let account = index.accounts.remove(ix);
        index.accounts.insert(0, account);
        index.active = Some(id);
        write_index(index);
        state.reset_session();
        state.token = Some(Some(token));
    });
    Ok(())
}

/// Keeps the switcher's name and picture current.
pub fn remember_profile(user: &CurrentUser) {
    with_state(|state| {
        let index = state.index();
        let Some(account) = index
            .accounts
            .iter_mut()
            .find(|account| account.id == user.id)
        else {
            return;
        };
        if account.name == user.name
            && account.username == user.username
            && account.avatar_url == user.avatar_url
        {
            return;
        }
        account.name = user.name.clone();
        account.username = user.username.clone();
        account.avatar_url = user.avatar_url.clone();
        write_index(index);
    });
}

/// Drops an account and hands back its token, for revoking. Ends the session if it was
/// the active one.
pub(super) fn remove_account(id: Id<UserMarker>) -> Option<String> {
    let token = read_stored_token(id);
    delete_stored_token(id);
    with_state(|state| {
        let index = state.index();
        index.accounts.retain(|account| account.id != id);
        let was_active = index.active == Some(id);
        if was_active {
            index.active = None;
        }
        write_index(index);
        if was_active {
            state.reset_session();
            state.token = Some(None);
        }
    });
    token
}

/// Forgets the account `expired` belongs to, so the next launch doesn't try it. Only if
/// it's still that account's token: a request sent with an old one can fail after the
/// user logged in again.
pub(super) fn forget_token(expired: &str) {
    let Some(id) = user_id_of(expired) else {
        return;
    };
    if read_stored_token(id).as_deref() == Some(expired) {
        remove_account(id);
    }
}

/// Must be called on the Tokio runtime: building the client spawns its ratelimiter there.
pub(super) fn client() -> anyhow::Result<Arc<HttpClient>> {
    // One lock throughout, so a switch can't pair the new session with the old token.
    with_state(|state| {
        if let Some(client) = &state.client {
            return Ok(client.clone());
        }
        let token = state
            .active_token()
            .ok_or_else(|| anyhow::anyhow!("No token found. Please log in first."))?;
        Ok(state
            .client
            .insert(Arc::new(HttpClient::new(token)))
            .clone())
    })
}
