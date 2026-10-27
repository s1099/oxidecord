//! The app's preset themes, baked in from `themes/` by `build.rs` and parsed at
//! startup into [`Themes`], the global the settings page picks from.
//!
//! gpui-component's [`ThemeRegistry`] only loads from a watched directory and
//! can't register a theme from memory, so presets are kept alongside it.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeConfig, ThemeRegistry, ThemeSet};
use gpui_kit::*;

use crate::platform::prefs;

include!(concat!(env!("OUT_DIR"), "/preset_themes.rs"));

const DEFAULT_PRESET: &str = "Zinc";

/// Corners for presets that don't set their own; gpui-component's default is
/// tighter than the app's (see [`crate::ui::depth::radius`]).
const RADIUS: Pixels = px(8.);
const RADIUS_LG: Pixels = px(14.);

/// Toast stack top offset, clear of the window controls.
const NOTIFICATION_TOP: Pixels = px(68.);

pub struct Themes {
    presets: Vec<Rc<ThemeConfig>>,
}

impl Global for Themes {}

/// Parses the baked-in themes and restores the last chosen one. Must run after
/// `gpui_kit::component::init`, which installs the globals this builds on.
pub fn init(cx: &mut App) {
    // Built-ins first, so there's always a way back to the stock look.
    let mut presets: Vec<Rc<ThemeConfig>> = vec![
        ThemeRegistry::global(cx).default_light_theme().clone(),
        ThemeRegistry::global(cx).default_dark_theme().clone(),
    ];

    for (file, source) in PRESET_THEME_FILES {
        match serde_json::from_str::<ThemeSet>(source) {
            // A file is a set: often a light and a dark variant, each its own theme.
            Ok(set) => presets.extend(set.themes.into_iter().map(Rc::new)),
            Err(err) => eprintln!("ignoring invalid preset theme {file}: {err}"),
        }
    }

    // Prefs store the name, so duplicates are ambiguous. First wins.
    let mut seen = std::collections::HashSet::new();
    presets.retain(|theme| seen.insert(theme.name.clone()));

    // Light before dark, then by name, for a stable order.
    presets.sort_by(|a, b| {
        a.mode
            .is_dark()
            .cmp(&b.mode.is_dark())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    cx.set_global(Themes { presets });

    // Default: Zinc, in the mode the system appearance picked during init.
    let name = prefs::load().theme.unwrap_or_else(|| {
        let mode = if Theme::global(cx).mode.is_dark() {
            "Dark"
        } else {
            "Light"
        };
        format!("{DEFAULT_PRESET} {mode}")
    });
    apply(&name, None, cx);
}

/// Secondary text that must stay readable (channel names, topics): presets'
/// `muted_foreground` is too dim, so this pulls it partway toward the foreground.
pub fn secondary_text(cx: &App) -> Hsla {
    let theme = cx.theme();
    theme.muted_foreground.blend(theme.foreground.opacity(0.35))
}

pub fn presets(cx: &App) -> &[Rc<ThemeConfig>] {
    &cx.global::<Themes>().presets
}

/// Matches a preset's name once one is picked; before that, a built-in's.
pub fn active_name(cx: &App) -> SharedString {
    Theme::global(cx).theme_name().clone()
}

/// Switches to the named preset and persists the choice.
pub fn activate(name: &str, window: &mut Window, cx: &mut App) {
    apply(name, Some(window), cx);
    prefs::update(|prefs| prefs.theme = Some(name.to_string()));
}

/// Unknown names are ignored: a stored preference can outlive its theme file.
fn apply(name: &str, window: Option<&mut Window>, cx: &mut App) {
    let Some(config) = presets(cx).iter().find(|theme| theme.name == name).cloned() else {
        return;
    };

    let theme = Theme::global_mut(cx);
    theme.apply_config(&config);
    if config.radius.is_none() {
        theme.radius = RADIUS;
    }
    if config.radius_lg.is_none() {
        theme.radius_lg = RADIUS_LG;
    }
    theme.notification.margins.top = NOTIFICATION_TOP;

    // Colours are read from the global at render, so repaint every window.
    match window {
        Some(window) => window.refresh(),
        None => cx.refresh_windows(),
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would pull in gpui's `test` macro over the standard one.
    use gpui_kit::component::ThemeSet;

    use super::PRESET_THEME_FILES;

    /// A theme file that fails to parse is silently dropped at startup.
    #[test]
    fn every_preset_file_parses() {
        assert!(!PRESET_THEME_FILES.is_empty(), "no themes were baked in");

        for (file, source) in PRESET_THEME_FILES {
            let set = serde_json::from_str::<ThemeSet>(source)
                .unwrap_or_else(|err| panic!("{file} failed to parse: {err}"));
            assert!(!set.themes.is_empty(), "{file} declares no themes");
        }
    }
}
