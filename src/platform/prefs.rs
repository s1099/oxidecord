//! User preferences that outlive a run, kept in a small JSON file in the
//! platform's config directory.
//!
//! Nothing here is essential to the app working, so every operation degrades to
//! a default instead of failing: an unreadable or malformed file reads as "no
//! preferences set", and a failed write is dropped.
//!
//! The file is read once and kept in memory, since some reads happen on every
//! channel switch on the foreground thread. Writes go through that copy, so
//! the file is only touched when something changes.

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
    /// Name of the selected theme, as it appears in the theme's JSON. `None`
    /// until the user picks one, which leaves the system appearance in charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Id of the microphone calls should use. `None` follows whichever device
    /// the system calls the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,
    /// Id of the speaker calls should play through. `None` follows whichever
    /// device the system calls the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    /// Channel categories the user has collapsed, keyed by guild id. Raw ids
    /// keep this module free of Discord types.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub collapsed_categories: BTreeMap<u64, Vec<u64>>,
    /// Emoji the user last chose from the picker, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_emojis: Vec<RecentEmoji>,
    /// The user closed the member list beside guild conversations.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub member_list_hidden: bool,
}

/// An emoji in [`Prefs::recent_emojis`]: a custom one by id, a unicode one as
/// its text. Untagged, so the JSON is just a number or a string.
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

/// Loads, edits, and saves in one step, so a caller changing one field doesn't
/// clear the ones it doesn't know about.
pub fn update(edit: impl FnOnce(&mut Prefs)) {
    let mut cache = cached();
    let prefs = cache.get_or_insert_with(read_file);
    edit(prefs);
    write_file(prefs);
}

fn cached() -> MutexGuard<'static, Option<Prefs>> {
    // A panic mid-edit leaves at worst a half-edited copy, still better than none
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What a preference stores survives the trip to disk's format: both
    /// device ids come back, and unset ones stay unset so an older file
    /// without them still reads.
    #[test]
    fn device_choices_round_trip() {
        let prefs = Prefs {
            input_device: Some("{0.0.1.00000000}.{guid}".into()),
            output_device: Some("{0.0.0.00000000}.{other-guid}".into()),
            ..Prefs::default()
        };
        let read: Prefs = serde_json::from_str(&serde_json::to_string(&prefs).expect("serialize"))
            .expect("valid");

        assert_eq!(
            read.input_device.as_deref(),
            Some("{0.0.1.00000000}.{guid}")
        );
        assert_eq!(
            read.output_device.as_deref(),
            Some("{0.0.0.00000000}.{other-guid}")
        );
        let legacy: Prefs =
            serde_json::from_str(r#"{"input_device":"wasapi:legacy","theme":"dark"}"#)
                .expect("existing preferences without an output choice remain readable");
        assert_eq!(legacy.output_device, None);
        assert_eq!(legacy.input_device.as_deref(), Some("wasapi:legacy"));
        assert_eq!(legacy.theme.as_deref(), Some("dark"));
    }
}
