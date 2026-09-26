//! The app's preset themes.
//!
//! `build.rs` bakes every JSON file in `themes/` into the binary, so the
//! presets travel with it and no theme folder has to sit beside the executable.
//! They're parsed once at startup into [`Themes`], the global the settings page
//! lists and picks from.
//!
//! gpui-component's own [`ThemeRegistry`] is only used for the two themes it
//! ships with; it loads from a watched directory and has no way to register a
//! theme from memory, so the presets are kept alongside it rather than in it.

use std::rc::Rc;

use gpui::*;
use gpui_component::{ActiveTheme as _, Theme, ThemeConfig, ThemeRegistry, ThemeSet};

use crate::platform::prefs;

include!(concat!(env!("OUT_DIR"), "/preset_themes.rs"));

/// The preset family used until one is picked, from `themes/zinc.json`.
const DEFAULT_PRESET: &str = "Zinc";

/// Corners for presets that don't set their own. gpui-component's defaults are
/// tighter than the rest of the UI (see [`crate::ui::depth::radius`]), and its
/// widgets — inputs, menus, dialogs — should match the app's own.
const RADIUS: Pixels = px(8.);
const RADIUS_LG: Pixels = px(14.);

/// Every theme the settings page can switch to, in the order it lists them.
pub struct Themes {
    presets: Vec<Rc<ThemeConfig>>,
}

impl Global for Themes {}

/// Parses the baked-in themes and restores the last chosen one.
///
/// Must run after `gpui_component::init`, which installs the [`Theme`] and
/// [`ThemeRegistry`] globals this builds on.
pub fn init(cx: &mut App) {
    // The built-in light and dark themes first, so there's always a way back to
    // the stock look.
    let mut presets: Vec<Rc<ThemeConfig>> = vec![
        ThemeRegistry::global(cx).default_light_theme().clone(),
        ThemeRegistry::global(cx).default_dark_theme().clone(),
    ];

    for (file, source) in PRESET_THEME_FILES {
        match serde_json::from_str::<ThemeSet>(source) {
            // A file holds a set: one JSON file often carries a light and a
            // dark variant, and each variant is a theme of its own here.
            Ok(set) => presets.extend(set.themes.into_iter().map(Rc::new)),
            Err(err) => eprintln!("ignoring invalid preset theme {file}: {err}"),
        }
    }

    // Names are the identity a preference is stored under, so a duplicate would
    // make the choice ambiguous. First one wins.
    let mut seen = std::collections::HashSet::new();
    presets.retain(|theme| seen.insert(theme.name.clone()));

    // Light before dark, then by name, so the list reads the same every launch.
    presets.sort_by(|a, b| {
        a.mode
            .is_dark()
            .cmp(&b.mode.is_dark())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    cx.set_global(Themes { presets });

    // Without a stored choice, Zinc — the palette the rest of the UI is designed
    // around — in whichever mode the system appearance picked during
    // `gpui_component::init`.
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

/// The presets, in display order.
/// Secondary text that's still meant to be read — channel names, a channel's
/// topic. Presets make `muted_foreground` dim enough for hints and timestamps,
/// which is too dim for these, so it's pulled part of the way toward the
/// foreground, stopping short of the messages themselves.
pub fn secondary_text(cx: &App) -> Hsla {
    let theme = cx.theme();
    theme.muted_foreground.blend(theme.foreground.opacity(0.35))
}

pub fn presets(cx: &App) -> &[Rc<ThemeConfig>] {
    &cx.global::<Themes>().presets
}

/// Name of the theme currently in use. Matches a preset's name once one has
/// been picked; before that it's whichever built-in the system appearance chose.
pub fn active_name(cx: &App) -> SharedString {
    Theme::global(cx).theme_name().clone()
}

/// Switches to the named preset and remembers it for the next launch.
pub fn activate(name: &str, window: &mut Window, cx: &mut App) {
    apply(name, Some(window), cx);
    prefs::update(|prefs| prefs.theme = Some(name.to_string()));
}

/// Applies a preset by name, ignoring names that aren't among them — a stored
/// preference can outlive the theme file it points at.
fn apply(name: &str, window: Option<&mut Window>, cx: &mut App) {
    let Some(config) = presets(cx).iter().find(|theme| theme.name == name).cloned() else {
        return;
    };

    // `apply_config` also sets the mode, so a dark preset takes the app dark.
    let theme = Theme::global_mut(cx);
    theme.apply_config(&config);
    if config.radius.is_none() {
        theme.radius = RADIUS;
    }
    if config.radius_lg.is_none() {
        theme.radius_lg = RADIUS_LG;
    }

    // Colours are read straight out of the global during render, so every open
    // window has to be repainted, not just the one the click came from.
    match window {
        Some(window) => window.refresh(),
        None => cx.refresh_windows(),
    }
}

#[cfg(test)]
mod tests {
    // Named imports rather than a glob: `use super::*` would pull in gpui's own
    // `test` attribute macro over the standard one.
    use gpui_component::ThemeSet;

    use super::PRESET_THEME_FILES;

    /// A theme file that doesn't parse is dropped at startup with only a line on
    /// stderr to show for it, so catch it here instead.
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
