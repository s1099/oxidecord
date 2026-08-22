//! User preferences that outlive a run, kept in a small JSON file in the
//! platform's config directory.
//!
//! Nothing here is essential to the app working, so every operation degrades to
//! a default instead of failing: an unreadable or malformed file reads as "no
//! preferences set", and a failed write is dropped.

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

const APP_DIR: &str = "oxidecord";
const FILE_NAME: &str = "prefs.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Prefs {
    /// Name of the selected theme, as it appears in the theme's JSON. `None`
    /// until the user picks one, which leaves the system appearance in charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// Reads the preferences file, or returns the defaults if it isn't there yet.
pub fn load() -> Prefs {
    file_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

/// Writes `prefs` back out, creating the config directory if needed.
pub fn save(prefs: &Prefs) {
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

/// Loads, edits, and saves in one step, so a caller changing one field doesn't
/// clear the ones it doesn't know about.
pub fn update(edit: impl FnOnce(&mut Prefs)) {
    let mut prefs = load();
    edit(&mut prefs);
    save(&prefs);
}

/// The per-user config directory, following each platform's convention.
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
