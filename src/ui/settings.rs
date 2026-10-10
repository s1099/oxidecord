//! The settings popup: gpui-component's [`Settings`] component — a page sidebar
//! beside the active page's groups — hosted in a dialog, opened from the
//! account panel.
//!
//! Both pages are real: themes picks a preset, and updates drives the
//! self-updater in [`crate::platform::updater`]. Typing [`DEBUG_CODE`] into the
//! search adds a third, [`gallery`], for the rest of the session. A logout
//! button sits under the page list.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, ThemeConfig, WindowExt as _,
    group_box::GroupBoxVariant,
    h_flex,
    input::{InputEvent, InputState},
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

use crate::platform::updater::{self, Status};
use crate::ui::button::Button;
use crate::ui::{depth, gallery, theme};

/// Size the popup aims for. Both are capped to the window with [`WINDOW_MARGIN`]
/// to spare, since the dialog is positioned from a fixed size and would
/// otherwise run off a small window.
const WIDTH: f32 = 900.;
const HEIGHT: f32 = 600.;
const WINDOW_MARGIN: f32 = 64.;

/// Width of the page sidebar inside the popup.
const SIDEBAR_WIDTH: f32 = 220.;

/// The dialog's own border, one pixel top and bottom, which sits inside the
/// height it's given. See [`open`] for why the content has to subtract it.
const DIALOG_BORDER: f32 = 2.;

/// A theme card: wide enough for a couple of words of theme name, over a
/// preview roughly the proportions of the app window.
const CARD_WIDTH: f32 = 148.;
const PREVIEW_HEIGHT: f32 = 84.;

/// What the logout button calls, supplied by whoever opens the popup.
type LogOutHandler = dyn Fn(&mut Window, &mut App);

/// Typed into the settings search, reveals the debug page.
const DEBUG_CODE: &str = "debug";

/// Opens the settings popup. Closed by the dialog's own close button, Escape,
/// or a click on the overlay. `on_log_out` runs, after the popup closes, when
/// the logout button is clicked.
pub fn open(
    window: &mut Window,
    cx: &mut App,
    on_log_out: impl Fn(&mut Window, &mut App) + 'static,
) {
    arm_debug_code(cx);
    let on_log_out: Rc<LogOutHandler> = Rc::new(on_log_out);

    window.open_dialog(cx, move |dialog, window, cx| {
        let debug = cx
            .try_global::<DebugCode>()
            .is_some_and(|code| code.unlocked);
        let viewport = window.viewport_size();
        let width = px(WIDTH).min(viewport.width - px(WINDOW_MARGIN));
        let height = px(HEIGHT).min(viewport.height - px(WINDOW_MARGIN));
        let inner_radius = dialog_inner_radius(cx);

        dialog
            // The component brings its own sidebar, header, and scrolling, so
            // the dialog is only the frame around it: no padding and no title.
            // `overflow_hidden` clips to the rectangle, not the rounded corners,
            // so the sidebar and footer round their own corners to match.
            .p_0()
            .border_color(depth::ring(cx))
            .w(width)
            .h(height)
            .overflow_hidden()
            .child(
                // The dialog puts its content in a block-layout scroll box, so
                // a percentage height has nothing definite to resolve against
                // and the whole component would collapse to the height of the
                // sidebar's two entries. It needs a pixel height, and that
                // height is the dialog's content box: what it was given, less
                // the border sitting inside it. Overshoot by even a pixel and
                // the dialog scrolls the popup — sidebar and all.
                div()
                    .relative()
                    .w_full()
                    .h(height - px(DIALOG_BORDER))
                    .child(
                        Settings::new("app-settings")
                            .small()
                            .with_group_variant(GroupBoxVariant::Outline)
                            .sidebar_width(px(SIDEBAR_WIDTH))
                            .sidebar_style(&StyleRefinement::default().rounded_l(inner_radius))
                            .page(themes_page())
                            .page(updates_page())
                            .when(debug, |this| this.page(gallery::page())),
                    )
                    .child(log_out_footer(on_log_out.clone(), cx)),
            )
    });
}

/// The logout button, pinned under the sidebar's page list. The component's
/// sidebar has a footer slot but [`Settings`] doesn't expose it, so this is
/// laid over the sidebar's bottom edge instead, at the width it opens at. The
/// page list scrolls under it, and is far too short to reach it anyway.
fn log_out_footer(on_log_out: Rc<LogOutHandler>, cx: &App) -> Div {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        // Clear of the sidebar's right border, so the divider stays unbroken.
        .w(px(SIDEBAR_WIDTH) - px(1.))
        .p_3()
        .rounded_bl(dialog_inner_radius(cx))
        .bg(cx.theme().sidebar)
        .border_t_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            Button::new("log-out")
                .w_full()
                .small()
                .danger()
                .icon(Icon::default().path("icons/log-out.svg"))
                .label("Log Out")
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    on_log_out(window, cx);
                }),
        )
}

/// The radius of the dialog's corners inside its one-pixel border, for content
/// that paints right up to them.
fn dialog_inner_radius(cx: &App) -> Pixels {
    (cx.theme().radius_lg - px(1.)).max(px(0.))
}

/// Watches the settings search for [`DEBUG_CODE`]. The unlock is never saved,
/// so it lasts until the app quits.
#[derive(Default)]
struct DebugCode {
    unlocked: bool,
    armed: bool,
    _observer: Option<Subscription>,
}

impl Global for DebugCode {}

/// The [`Settings`] component builds its search input privately and offers no
/// way to read it, so the only way in is to catch the input being created.
/// Every input in the app goes through the same hook, so it's armed just
/// before the dialog opens and claims the next one made: the search box, which
/// the component creates on its first render, ahead of any page content.
fn arm_debug_code(cx: &mut App) {
    if !cx.has_global::<DebugCode>() {
        let observer = cx.observe_new(|_: &mut InputState, window, cx| {
            let armed = &mut cx.global_mut::<DebugCode>().armed;
            if !std::mem::take(armed) {
                return;
            }
            let Some(window) = window else {
                return;
            };
            let search = cx.entity();
            cx.subscribe_in(&search, window, |search, _, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && search.value().trim().eq_ignore_ascii_case(DEBUG_CODE)
                {
                    cx.global_mut::<DebugCode>().unlocked = true;
                    // Left in place, the code would filter every page out,
                    // the new one included — its content is custom-rendered,
                    // which the search never matches.
                    search.set_value("", window, cx);
                }
            })
            .detach();
        });
        cx.set_global(DebugCode {
            _observer: Some(observer),
            ..Default::default()
        });
    }

    let code = cx.global_mut::<DebugCode>();
    code.armed = !code.unlocked;
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
    // `overflow_hidden` clips to the rectangle, not the rounded corners, so the
    // children that reach the corners round themselves to sit inside the border.
    let inner_radius = (cx.theme().radius - px(1.)).max(px(0.));

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
                .child(
                    div()
                        .w_full()
                        .h(px(12.))
                        .rounded_t(inner_radius)
                        .bg(title_bar),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .child(
                            div()
                                .w(px(34.))
                                .h_full()
                                .rounded_bl(inner_radius)
                                .bg(sidebar),
                        )
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
                .item(SettingItem::new("Current version", version_field()))
                .item(SettingItem::new("Check for updates", update_field())),
        )
}

fn version_field() -> SettingField<SharedString> {
    SettingField::render(|_: &RenderOptions, _window: &mut Window, cx: &mut App| {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(updater::CURRENT_VERSION)
    })
}

/// The updater's whole interface: where it has got to, and the one action that
/// makes sense from there.
fn update_field() -> SettingField<SharedString> {
    SettingField::render(|_: &RenderOptions, _window: &mut Window, cx: &mut App| {
        let status = updater::status(cx);
        let (message, tone) = status_message(&status, cx);

        h_flex()
            .gap_3()
            .items_center()
            .when_some(message, |this, message| {
                this.child(div().text_sm().text_color(tone).child(message))
            })
            .children(action_button(&status))
    })
}

/// What the current status reads as, and the colour it reads in. `None` where
/// the button alone already says it.
fn status_message(status: &Status, cx: &App) -> (Option<SharedString>, Hsla) {
    let muted = cx.theme().muted_foreground;
    match status {
        Status::Idle => (None, muted),
        Status::Checking => (Some("Checking…".into()), muted),
        Status::UpToDate => (Some("You're on the latest version.".into()), muted),
        Status::Available { version, .. } => (
            Some(format!("Version {version} is available.").into()),
            cx.theme().foreground,
        ),
        Status::Downloading { percent, .. } => {
            (Some(format!("Downloading… {percent}%").into()), muted)
        }
        Status::Ready { version } => (
            Some(format!("Version {version} is ready. Restart to install it.").into()),
            cx.theme().success,
        ),
        Status::Failed(error) => (Some(error.clone()), cx.theme().danger),
    }
}

/// The action the current status leads to. Work in flight leaves a disabled
/// button in place, so the row doesn't reflow while it runs.
fn action_button(status: &Status) -> Option<Button> {
    if !updater::supported() {
        return None;
    }

    Some(match status {
        Status::Checking | Status::Downloading { .. } => Button::new("update-action")
            .label("Working")
            .loading(true)
            .disabled(true),
        Status::Available { .. } => Button::new("update-action")
            .label("Download")
            .primary()
            .on_click(|_, _window, cx| updater::download(cx)),
        Status::Ready { .. } => Button::new("update-action")
            // The swap can only happen once this process is gone, so restarting
            // is the install rather than a step before it.
            .label("Restart to update")
            .primary()
            .on_click(|_, _window, cx| updater::install(cx)),
        Status::Idle | Status::UpToDate | Status::Failed(_) => Button::new("update-action")
            .label("Check for updates")
            .outline()
            .on_click(|_, _window, cx| updater::check(cx)),
    })
}
