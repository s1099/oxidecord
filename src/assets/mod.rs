//! App asset source: serves the app's own icons and falls back to the
//! assets bundled with gpui-component for everything else.

pub mod icons;

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

use icons::EXTRA_ICONS;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, svg)) = EXTRA_ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items: Vec<SharedString> = EXTRA_ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect();
        items.extend(gpui_component_assets::Assets.list(path)?);
        Ok(items)
    }
}
