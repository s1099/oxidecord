//! The settings popup: gpui-component's [`Settings`] hosted in a dialog.
//!
//! Pages: themes, and updates (drives [`crate::platform::updater`]). Typing
//! [`DEBUG_CODE`] into the search adds [`gallery`] for the session.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, ThemeConfig, WindowExt as _,
    group_box::GroupBoxVariant,
    h_flex,
    input::{InputEvent, InputState},
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform::updater::{self, Status};
use crate::ui::button::Button;
use crate::ui::{depth, gallery, theme};

/// Popup size, capped to the window less [`WINDOW_MARGIN`]: the dialog is
/// positioned from a fixed size and would run off a small window.
const WIDTH: f32 = 900.;
const HEIGHT: f32 = 600.;
const WINDOW_MARGIN: f32 = 64.;

const SIDEBAR_WIDTH: f32 = 220.;

/// The dialog's border (1px top and bottom) sits inside its height; see [`open`].
const DIALOG_BORDER: f32 = 2.;

const CARD_WIDTH: f32 = 148.;
const PREVIEW_HEIGHT: f32 = 84.;

type LogOutHandler = dyn Fn(&mut Window, &mut App);

const DEBUG_CODE: &str = "debug";

/// Opens the settings popup. `on_log_out` runs after it closes, when logout is clicked.
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
            // The component brings its own chrome, so the dialog is just a frame.
            // `overflow_hidden` clips to the rectangle, not the rounded corners,
            // so the sidebar and footer round their own.
            .p_0()
            .border_color(depth::ring(cx))
            .w(width)
            .h(height)
            .overflow_hidden()
            .child(
                // The dialog's content is a block-layout scroll box, so a percentage
                // height has nothing to resolve against and the component collapses
                // to the sidebar's height. Use the content box in pixels (given
                // height less the inner border); overshoot by a pixel and the
                // dialog scrolls the whole popup.
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

/// The logout button. [`Settings`] doesn't expose the sidebar's footer slot, so
/// this is laid over the sidebar's bottom edge at its opening width.
fn log_out_footer(on_log_out: Rc<LogOutHandler>, cx: &App) -> Div {
    div()
        .absolute()
        .bottom_0()
        .left_0()
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

/// Corner radius inside the dialog's border, for content painting up to it.
fn dialog_inner_radius(cx: &App) -> Pixels {
    (cx.theme().radius_lg - px(1.)).max(px(0.))
}

/// Watches the settings search for [`DEBUG_CODE`]. The unlock is not persisted.
#[derive(Default)]
struct DebugCode {
    unlocked: bool,
    armed: bool,
    _observer: Option<Subscription>,
}

impl Global for DebugCode {}

/// [`Settings`] builds its search input privately, so catch it on creation:
/// every input goes through one hook, armed just before the dialog opens, and
/// the next one made is the search box (created on first render, before pages).
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
                    // Left in, the code would filter out every page, including the
                    // new one (custom-rendered, which search never matches).
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

fn theme_grid(cx: &mut App) -> Div {
    let active = theme::active_name(cx);
    let presets: Vec<Rc<ThemeConfig>> = theme::presets(cx).to_vec();

    // A loop, not `children`: each card needs `&mut App`, which a closure can't hand back.
    let mut grid = div().flex().flex_wrap().gap_3();
    for preset in presets {
        let selected = preset.name == active;
        grid = grid.child(theme_card(preset, selected, cx));
    }
    grid
}

/// A miniature of the app window in the preset's colours; the selected card gets an accent outline.
fn theme_card(preset: Rc<ThemeConfig>, selected: bool, cx: &mut App) -> Stateful<Div> {
    let colors = &preset.colors;
    let name = preset.name.clone();

    // Every colour is optional; fall back to another from the same theme.
    let background = color(&colors.background, cx.theme().background);
    let foreground = color(&colors.foreground, cx.theme().foreground);
    let sidebar = color(&colors.sidebar, background);
    let title_bar = color(&colors.title_bar, sidebar);
    let primary = color(&colors.primary, foreground);
    let outline = color(&colors.border, foreground.alpha(0.2));
    // Corner children round themselves instead of using `overflow_hidden`: it
    // doesn't clip rounded corners, and gpui rounds the clip out to whole device
    // pixels, leaving a title-bar sliver when a card scrolls past the list edge.
    let inner_radius = (cx.theme().radius - px(1.)).max(px(0.));

    card_frame(name.clone(), selected, cx)
        .child(
            div()
                .w_full()
                .h(px(PREVIEW_HEIGHT))
                .rounded(cx.theme().radius)
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

fn preview_line(width: Pixels, color: Hsla) -> Div {
    div().w(width).h(px(4.)).rounded_full().bg(color)
}

/// Parses an optional theme colour string; non-hex values (a gradient, say) fall
/// back too, since a preview is a flat fill.
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

/// `None` where the button alone already says it.
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

/// Work in flight leaves a disabled button so the row doesn't reflow.
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
            // The swap happens only once this process exits, so restarting is the install.
            .label("Restart to update")
            .primary()
            .on_click(|_, _window, cx| updater::install(cx)),
        Status::Idle | Status::UpToDate | Status::Failed(_) => Button::new("update-action")
            .label("Check for updates")
            .outline()
            .on_click(|_, _window, cx| updater::check(cx)),
    })
}
