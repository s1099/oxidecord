//! User preferences in a JSON file in the config directory.
//!
//! Nothing here is essential, so a bad file reads as defaults and a failed
//! write is dropped. The file is cached in memory because some reads happen on
//! every channel switch on the foreground thread.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};

const APP_DIR: &str = "oxidecord";
const FILE_NAME: &str = "prefs.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Prefs {
    /// `None` leaves the system appearance in charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// `None` follows the system default device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,
    /// `None` follows the system default device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    /// Keyed by guild id. Raw ids keep this module free of Discord types.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub collapsed_categories: BTreeMap<u64, Vec<u64>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_emojis: Vec<RecentEmoji>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub member_list_hidden: bool,
}

/// Untagged, so the JSON is just a number (custom) or a string (unicode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecentEmoji {
    Custom(u64),
    Unicode(String),
}

static CACHE: Mutex<Option<Prefs>> = Mutex::new(None);

pub fn load() -> Prefs {
    cached().get_or_insert_with(read_file).clone()
}

pub fn save(prefs: &Prefs) {
    *cached() = Some(prefs.clone());
    write_file(prefs);
}

/// Edits one field without clobbering the rest.
pub fn update(edit: impl FnOnce(&mut Prefs)) {
    let mut cache = cached();
    let prefs = cache.get_or_insert_with(read_file);
    edit(prefs);
    write_file(prefs);
}

fn cached() -> MutexGuard<'static, Option<Prefs>> {
    // A panic mid-edit leaves a half-edited copy at worst, better than none.
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_file() -> Prefs {
    file_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

fn write_file(prefs: &Prefs) {
    let Some(path) = file_path() else {
        return;
    };
    let Ok(contents) = serde_json::to_string_pretty(prefs) else {
        return;
    };

    if let Some(dir) = path.parent()
        && fs::create_dir_all(dir).is_err()
    {
        return;
    }
    _ = fs::write(path, contents);
}

fn file_path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            })
    };

    Some(dir.join(APP_DIR).join(FILE_NAME))
}
