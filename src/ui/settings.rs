//! The settings popup: gpui-component's [`Settings`] component — a page sidebar
//! beside the active page's groups — hosted in a dialog, opened from the
//! account panel.
//!
//! The themes page is real; the updates page is still a placeholder, and the
//! structure it uses is the one a filled-in page has.

use std::rc::Rc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, ThemeConfig, WindowExt as _,
    group_box::GroupBoxVariant,
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

use crate::ui::theme;

/// Size the popup aims for. Both are capped to the window with [`WINDOW_MARGIN`]
/// to spare, since the dialog is positioned from a fixed size and would
/// otherwise run off a small window.
const WIDTH: f32 = 900.;
const HEIGHT: f32 = 600.;
const WINDOW_MARGIN: f32 = 64.;

/// Width of the page sidebar inside the popup.
const SIDEBAR_WIDTH: f32 = 220.;

/// A theme card: wide enough for a couple of words of theme name, over a
/// preview roughly the proportions of the app window.
const CARD_WIDTH: f32 = 148.;
const PREVIEW_HEIGHT: f32 = 84.;

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
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("Preset themes")
                .item(SettingItem::render(|_: &RenderOptions, _window, cx| {
                    theme_grid(cx)
                })),
        )
}

/// Every preset laid out as a wrapping grid of preview cards.
fn theme_grid(cx: &mut App) -> Div {
    let active = theme::active_name(cx);
    // Cloned out of the global so the cards can be built against `&mut App`.
    let presets: Vec<Rc<ThemeConfig>> = theme::presets(cx).to_vec();

    // Built up in a loop rather than with `children`, since each card needs
    // `&mut App` and a closure can't hand it back out.
    let mut grid = div().flex().flex_wrap().gap_3();
    for preset in presets {
        let selected = preset.name == active;
        grid = grid.child(theme_card(preset, selected, cx));
    }
    grid
}

/// One preset: a miniature of the app window in that theme's colours with the
/// theme's name under it. The selected card is outlined in the accent colour.
fn theme_card(preset: Rc<ThemeConfig>, selected: bool, cx: &mut App) -> Stateful<Div> {
    let colors = &preset.colors;
    let name = preset.name.clone();

    // Every colour in a theme file is optional, so each one falls back to
    // another colour from the same theme rather than to a fixed value.
    let background = color(&colors.background, cx.theme().background);
    let foreground = color(&colors.foreground, cx.theme().foreground);
    let sidebar = color(&colors.sidebar, background);
    let title_bar = color(&colors.title_bar, sidebar);
    let primary = color(&colors.primary, foreground);
    let outline = color(&colors.border, foreground.alpha(0.2));

    card_frame(name.clone(), selected, cx)
        .child(
            // The miniature: title bar across the top, sidebar down the left,
            // and a few lines standing in for content beside it.
            div()
                .w_full()
                .h(px(PREVIEW_HEIGHT))
                .rounded(cx.theme().radius)
                .overflow_hidden()
                .border_1()
                .border_color(outline)
                .bg(background)
                .flex()
                .flex_col()
                .child(div().w_full().h(px(12.)).bg(title_bar))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .child(div().w(px(34.)).h_full().bg(sidebar))
                        .child(
                            div()
                                .flex_1()
                                .p_2()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(preview_line(px(48.), foreground.alpha(0.75)))
                                .child(preview_line(px(34.), foreground.alpha(0.45)))
                                .child(preview_line(px(42.), primary)),
                        ),
                ),
        )
        .child(
            div()
                .w_full()
                .text_xs()
                .truncate()
                .text_color(if selected {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(name),
        )
}

/// The clickable frame around a card's preview and label.
fn card_frame(name: SharedString, selected: bool, cx: &mut App) -> Stateful<Div> {
    let hover = cx.theme().accent;
    let border = if selected {
        cx.theme().primary
    } else {
        cx.theme().transparent
    };

    div()
        .id(ElementId::Name(name.clone()))
        .w(px(CARD_WIDTH))
        .p_1p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .rounded(cx.theme().radius_lg)
        .border_2()
        .border_color(border)
        .cursor_pointer()
        .hover(move |this| this.bg(hover))
        .on_click(move |_, window, cx| theme::activate(&name, window, cx))
}

/// A stand-in for a line of text inside a preview.
fn preview_line(width: Pixels, color: Hsla) -> Div {
    div().w(width).h(px(4.)).rounded_full().bg(color)
}

/// Resolves an optional theme colour, which the JSON holds as a string, to the
/// colour to paint. Anything that isn't a plain hex value — a gradient, say —
/// falls back too, since a preview swatch is a flat fill.
fn color(value: &Option<SharedString>, fallback: Hsla) -> Hsla {
    value
        .as_ref()
        .and_then(|value| Rgba::try_from(value.as_ref()).ok())
        .map(Hsla::from)
        .unwrap_or(fallback)
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
