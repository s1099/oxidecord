//! The settings popup: gpui-component's [`Settings`] component — a page sidebar
//! beside the active page's groups — hosted in a dialog, opened from the
//! account panel.
//!
//! Both pages are placeholders for now. The structure is the real one, so
//! filling a page in is a matter of swapping its placeholder items for fields.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    group_box::GroupBoxVariant,
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

/// Size the popup aims for. Both are capped to the window with [`WINDOW_MARGIN`]
/// to spare, since the dialog is positioned from a fixed size and would
/// otherwise run off a small window.
const WIDTH: f32 = 900.;
const HEIGHT: f32 = 600.;
const WINDOW_MARGIN: f32 = 64.;

/// Width of the page sidebar inside the popup.
const SIDEBAR_WIDTH: f32 = 220.;

/// Opens the settings popup. Closed by the dialog's own close button, Escape,
/// or a click on the overlay.
pub fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, window, _cx| {
        let viewport = window.viewport_size();
        let width = px(WIDTH).min(viewport.width - px(WINDOW_MARGIN));
        let height = px(HEIGHT).min(viewport.height - px(WINDOW_MARGIN));

        dialog
            // The component brings its own sidebar, header, and scrolling, so
            // the dialog is only the frame around it: no padding, no title, and
            // clipped so the sidebar doesn't square off the rounded corners.
            .p_0()
            .w(width)
            .h(height)
            .overflow_hidden()
            .child(
                div().w_full().h(height).child(
                    Settings::new("app-settings")
                        .small()
                        .with_group_variant(GroupBoxVariant::Outline)
                        .sidebar_width(px(SIDEBAR_WIDTH))
                        .page(themes_page())
                        .page(updates_page()),
                ),
            )
    });
}

fn themes_page() -> SettingPage {
    SettingPage::new("Themes")
        .description("Choose how Oxidecord looks.")
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("Appearance")
                .description("Colour scheme and accent.")
                .item(
                    SettingItem::new("Theme", placeholder_field())
                        .description("Pick a built-in light or dark theme."),
                )
                .item(
                    SettingItem::new("Custom themes", placeholder_field())
                        .description("Load themes from the themes folder."),
                ),
        )
}

fn updates_page() -> SettingPage {
    SettingPage::new("Updates")
        .description("Keep Oxidecord up to date.")
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("Software update")
                .description("How new versions are found and installed.")
                .item(
                    SettingItem::new("Current version", placeholder_field())
                        .description("The version you're running, and when it was last checked."),
                )
                .item(
                    SettingItem::new("Automatic updates", placeholder_field())
                        .description("Download and install new versions in the background."),
                ),
        )
}

/// Stands in for a page's real control until it's built, so the rows lay out
/// the way they will once the fields land.
fn placeholder_field() -> SettingField<SharedString> {
    SettingField::render(|_: &RenderOptions, _window: &mut Window, cx: &mut App| {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("Coming soon")
    })
}
