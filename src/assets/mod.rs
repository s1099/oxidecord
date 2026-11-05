//! App asset source: serves the app's own icons and falls back to the
//! assets bundled with gpui-component for everything else.

pub mod icons;

use std::borrow::Cow;

use anyhow::Result;
use gpui_kit::{AssetSource, SharedString};

use icons::EXTRA_ICONS;

pub struct Assets;

/// One of the app's own icons, for drawing outside gpui.
pub fn icon_svg(path: &str) -> Option<&'static str> {
    EXTRA_ICONS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, svg)| *svg)
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(svg) = icon_svg(path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items: Vec<SharedString> = EXTRA_ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect();
        items.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(items)
    }
}
