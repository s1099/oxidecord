//! The debug page: every UI primitive the app builds with, drawn in the active
//! theme on one scrolling page, as a reference while working on the design.
//!
//! Nothing here is wired to real state — the controls keep throwaway state of
//! their own so they can be clicked through. The page stays hidden until it's
//! asked for through the settings search; see [`crate::ui::settings`].

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, Size,
    alert::Alert,
    avatar::{Avatar, AvatarGroup},
    badge::Badge,
    checkbox::Checkbox,
    divider::Divider,
    h_flex,
    input::{Input, InputState},
    kbd::Kbd,
    label::Label,
    progress::Progress,
    radio::{Radio, RadioGroup},
    setting::{RenderOptions, SettingGroup, SettingItem, SettingPage},
    skeleton::Skeleton,
    slider::{Slider, SliderState},
    spinner::Spinner,
    switch::Switch,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};

use crate::ui::button::Button;
use crate::ui::depth::{self, Finish, Level, Lit as _, media_shadow, radius};

/// Every size a [`Sizable`](gpui_component::Sizable) widget comes in, smallest first.
const SIZES: [(&str, Size); 4] = [
    ("xsmall", Size::XSmall),
    ("small", Size::Small),
    ("medium", Size::Medium),
    ("large", Size::Large),
];

/// Width of a colour swatch, which also bounds its token name.
const SWATCH_WIDTH: f32 = 104.;

/// Width given to widgets that would otherwise stretch across the page.
const FIELD_WIDTH: f32 = 260.;

pub fn page() -> SettingPage {
    SettingPage::new("Debug")
        .description("Every UI primitive, drawn in the active theme.")
        .resettable(false)
        .groups([
            section("Colors", colors),
            section("Typography", typography),
            section("Shape and depth", shape),
            section("Buttons", buttons),
            section("Form controls", controls),
            section("Feedback", feedback),
            section("Navigation", navigation),
            section("Avatars", avatars),
            section("Icons", icons),
        ])
}

/// A titled group holding one custom-rendered block. The page's sidebar lists
/// the titles, so each section is a jump target.
fn section<E: IntoElement>(
    title: &'static str,
    render: impl Fn(&mut Window, &mut App) -> E + 'static,
) -> SettingGroup {
    SettingGroup::new().title(title).item(SettingItem::render(
        move |_: &RenderOptions, window, cx| render(window, cx),
    ))
}

/// A captioned sub-block within a section.
fn specimen(caption: &'static str, content: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(caption),
        )
        .child(content)
}

fn row() -> Div {
    h_flex().flex_wrap().items_center().gap_2()
}

fn stack() -> Div {
    v_flex().w_full().gap_5()
}

/// State for a demo control that lives as long as the control is on screen.
fn demo_state<S: 'static>(
    id: &'static str,
    window: &mut Window,
    cx: &mut App,
    init: impl FnOnce(&mut Window, &mut Context<S>) -> S,
) -> Entity<S> {
    window.use_keyed_state(SharedString::from(id), cx, init)
}

/// A click handler that stores what the control reports into its state.
fn store<T: Copy + 'static>(state: &Entity<T>) -> impl Fn(&T, &mut Window, &mut App) + 'static {
    let state = state.clone();
    move |value, _, cx| {
        state.update(cx, |current, cx| {
            *current = *value;
            cx.notify();
        })
    }
}

macro_rules! tokens {
    ($theme:expr; $($name:ident),* $(,)?) => {
        vec![$((stringify!($name), $theme.$name)),*]
    };
}

fn colors(_: &mut Window, cx: &mut App) -> Div {
    let theme = cx.theme();
    let groups = [
        (
            "Surfaces",
            tokens!(theme; background, foreground, muted, muted_foreground, accent,
                accent_foreground, popover, popover_foreground, border, input, ring,
                selection, overlay),
        ),
        (
            "Chrome",
            tokens!(theme; title_bar, title_bar_border, sidebar, sidebar_foreground,
                sidebar_accent, sidebar_accent_foreground, sidebar_border, window_border,
                tab_bar, tab, tab_active, list, list_hover, list_active, scrollbar_thumb),
        ),
        (
            "Intents",
            tokens!(theme; primary, primary_hover, primary_active, primary_foreground,
                secondary, secondary_hover, secondary_active, secondary_foreground, danger,
                danger_hover, danger_foreground, success, success_foreground, warning,
                warning_foreground, info, info_foreground, link),
        ),
        (
            "Palette",
            tokens!(theme; red, red_light, green, green_light, blue, blue_light, yellow,
                yellow_light, magenta, magenta_light, cyan, cyan_light, chart_1, chart_2,
                chart_3, chart_4, chart_5),
        ),
    ];

    let mut block = stack();
    for (caption, swatches) in groups {
        let grid = div().flex().flex_wrap().gap_3().children(
            swatches
                .into_iter()
                .map(|(name, color)| swatch(name, color, cx)),
        );
        block = block.child(specimen(caption, grid, cx));
    }
    block
}

fn swatch(name: &'static str, color: Hsla, cx: &App) -> Div {
    v_flex()
        .w(px(SWATCH_WIDTH))
        .gap_1()
        .child(
            div()
                .h(px(40.))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(color),
        )
        .child(div().text_xs().truncate().child(name))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hex(color)),
        )
}

/// A colour as `#rrggbb`, with the alpha byte appended when it isn't opaque.
fn hex(color: Hsla) -> SharedString {
    let rgba = color.to_rgb();
    let byte = |channel: f32| (channel * 255.).round() as u8;
    let mut hex = format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    );
    if rgba.a < 1. {
        hex.push_str(&format!("{:02x}", byte(rgba.a)));
    }
    hex.into()
}

fn typography(_: &mut Window, cx: &mut App) -> Div {
    let sample = "The quick brown fox jumps over the lazy dog";
    let theme = cx.theme();

    let sizes = v_flex()
        .gap_1()
        .child(div().text_xs().child(format!("text_xs — {sample}")))
        .child(div().text_sm().child(format!("text_sm — {sample}")))
        .child(div().text_base().child(format!("text_base — {sample}")))
        .child(div().text_lg().child(format!("text_lg — {sample}")))
        .child(div().text_xl().child(format!("text_xl — {sample}")))
        .child(div().text_2xl().child("text_2xl"))
        .child(div().text_3xl().child("text_3xl"));

    let weights = v_flex().gap_1().children(
        [
            ("light", FontWeight::LIGHT),
            ("normal", FontWeight::NORMAL),
            ("medium", FontWeight::MEDIUM),
            ("semibold", FontWeight::SEMIBOLD),
            ("bold", FontWeight::BOLD),
        ]
        .map(|(name, weight)| {
            div()
                .font_weight(weight)
                .child(format!("{name} — {sample}"))
        }),
    );

    let tones = v_flex()
        .gap_1()
        .child(div().text_color(theme.foreground).child("foreground"))
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child("muted_foreground"),
        )
        .child(div().text_color(theme.link).underline().child("link"))
        .child(div().text_color(theme.danger).child("danger"))
        .child(div().text_color(theme.success).child("success"))
        .child(div().text_color(theme.warning).child("warning"));

    let mono = div()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .child("let mono = \"monospace\"; // mono_font_family");

    let label = Label::new("Label").secondary("with secondary text");

    let truncated = div().w(px(FIELD_WIDTH)).truncate().child(sample);

    stack()
        .child(specimen("Sizes", sizes, cx))
        .child(specimen("Weights", weights, cx))
        .child(specimen("Tones", tones, cx))
        .child(specimen("Monospace", mono, cx))
        .child(specimen("Label", label, cx))
        .child(specimen("Truncated", truncated, cx))
}

fn shape(_: &mut Window, cx: &mut App) -> Div {
    let theme = cx.theme();
    let filled = || {
        div()
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
    };

    let radii = row()
        .gap_6()
        .child(tile("ITEM", filled().rounded(radius::ITEM), cx))
        .child(tile("CONTROL", filled().rounded(radius::CONTROL), cx))
        .child(tile("CARD", filled().rounded(radius::CARD), cx))
        .child(tile("full", filled().rounded_full(), cx));

    // Each level on the fill it's meant for. Padded so the overlay's long
    // shadow isn't cut off by the section's edge.
    let size = px(64.);
    let lit = |bg: Hsla, level: Level, finish: Finish| {
        div().bg(bg).lit(level, finish, size, radius::CONTROL, cx)
    };
    let levels = row()
        .gap_6()
        .p_4()
        .child(tile(
            "Raised",
            lit(theme.secondary, Level::Raised, Finish::Subtle),
            cx,
        ))
        .child(tile(
            "RaisedStrong",
            lit(theme.primary, Level::RaisedStrong, Finish::Subtle),
            cx,
        ))
        .child(tile(
            "Pressed",
            lit(theme.secondary, Level::Pressed, Finish::Matte),
            cx,
        ))
        .child(tile("Surface", depth::card(radius::CARD, cx), cx))
        .child(tile(
            "Overlay",
            lit(theme.popover, Level::Overlay, Finish::Subtle)
                .border_1()
                .border_color(depth::ring(cx)),
            cx,
        ))
        .child(tile(
            "media_shadow",
            div()
                .bg(theme.popover)
                .rounded(radius::ITEM)
                .shadow(media_shadow()),
            cx,
        ));

    stack()
        .child(specimen("Corner radius", radii, cx))
        .child(specimen("Depth", levels, cx))
}

/// A square sample of a shape or shadow, named underneath.
fn tile(name: &'static str, shape: Div, cx: &App) -> Div {
    v_flex()
        .items_center()
        .gap_2()
        .child(shape.size_16())
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(name),
        )
}

fn buttons(_: &mut Window, cx: &mut App) -> Div {
    let labelled = row()
        .child(Button::new("variant-primary").label("Primary").primary())
        .child(Button::new("variant-secondary").label("Secondary"))
        .child(Button::new("variant-outline").label("Outline").outline())
        .child(Button::new("variant-danger").label("Danger").danger())
        .child(Button::new("variant-ghost").label("Ghost").ghost())
        .child(Button::new("variant-link").label("Link").link());

    let icons = row()
        .child(icon_button("icon-primary").primary())
        .child(icon_button("icon-secondary"))
        .child(icon_button("icon-outline").outline())
        .child(icon_button("icon-danger").danger())
        .child(icon_button("icon-ghost").ghost());

    let sizes = row().children(SIZES.map(|(name, size)| {
        Button::new(SharedString::from(format!("size-{name}")))
            .icon(IconName::Plus)
            .label(name)
            .outline()
            .with_size(size)
    }));

    let icon_sizes = row().children(SIZES.map(|(name, size)| {
        icon_button(SharedString::from(format!("icon-size-{name}")))
            .outline()
            .with_size(size)
    }));

    let states = row()
        .child(
            Button::new("state-disabled")
                .label("Disabled")
                .primary()
                .disabled(true),
        )
        .child(
            Button::new("state-loading")
                .label("Loading")
                .outline()
                .loading(true),
        )
        .child(
            Button::new("state-selected")
                .label("Selected")
                .outline()
                .selected(true),
        )
        .child(
            Button::new("state-ghost-selected")
                .icon(Icon::default().path("icons/mic-off.svg"))
                .ghost()
                .selected(true)
                .tooltip("Ghost, selected"),
        );

    stack()
        .child(specimen("Variants", labelled, cx))
        .child(specimen("Icon only, with tooltip", icons, cx))
        .child(specimen("Sizes", sizes, cx))
        .child(specimen("Icon sizes", icon_sizes, cx))
        .child(specimen(
            "States (hold one down to see it pressed)",
            states,
            cx,
        ))
}

fn icon_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).icon(IconName::Settings).tooltip("Tooltip")
}

fn controls(window: &mut Window, cx: &mut App) -> Div {
    let input = demo_state("input", window, cx, |window, cx| {
        InputState::new(window, cx).placeholder("Placeholder text")
    });
    let filled = demo_state("input-filled", window, cx, |window, cx| {
        InputState::new(window, cx).default_value("Some text")
    });
    let disabled = demo_state("input-disabled", window, cx, |window, cx| {
        InputState::new(window, cx).default_value("Disabled")
    });
    let small = demo_state("input-small", window, cx, |window, cx| {
        InputState::new(window, cx).placeholder("Small")
    });
    let large = demo_state("input-large", window, cx, |window, cx| {
        InputState::new(window, cx).placeholder("Large")
    });
    let checked = demo_state("checkbox", window, cx, |_, _| true);
    let switched = demo_state("switch", window, cx, |_, _| true);
    let radio = demo_state("radio", window, cx, |_, _| 0usize);
    let slider = demo_state("slider", window, cx, |_, _| {
        SliderState::new().min(0.).max(100.).default_value(40.)
    });

    let inputs = v_flex()
        .w(px(FIELD_WIDTH))
        .gap_2()
        .child(Input::new(&input))
        .child(Input::new(&filled).cleanable(true))
        .child(Input::new(&disabled).disabled(true))
        .child(Input::new(&small).small())
        .child(Input::new(&large).large());

    let checkboxes = row()
        .gap_4()
        .child(
            Checkbox::new("checkbox")
                .label("Interactive")
                .checked(*checked.read(cx))
                .on_click(store(&checked)),
        )
        .child(Checkbox::new("checkbox-off").label("Unchecked"))
        .child(
            Checkbox::new("checkbox-disabled")
                .label("Disabled")
                .checked(true)
                .disabled(true),
        );

    let switches = row()
        .gap_4()
        .child(
            Switch::new("switch")
                .label("Interactive")
                .checked(*switched.read(cx))
                .on_click(store(&switched)),
        )
        .child(Switch::new("switch-off").label("Off"))
        .child(
            Switch::new("switch-disabled")
                .label("Disabled")
                .checked(true)
                .disabled(true),
        )
        .child(Switch::new("switch-small").checked(true).small());

    let radios = RadioGroup::horizontal("radio")
        .selected_index(Some(*radio.read(cx)))
        .on_click(store(&radio))
        .children(["Online", "Idle", "Do not disturb"])
        .child(
            Radio::new("radio-disabled")
                .label("Disabled")
                .disabled(true),
        );

    let sliders = v_flex()
        .w(px(FIELD_WIDTH))
        .gap_4()
        .child(Slider::new(&slider))
        .child(Slider::new(&slider).disabled(true));

    stack()
        .child(specimen("Input", inputs, cx))
        .child(specimen("Checkbox", checkboxes, cx))
        .child(specimen("Switch", switches, cx))
        .child(specimen("Radio", radios, cx))
        .child(specimen("Slider", sliders, cx))
}

fn feedback(_: &mut Window, cx: &mut App) -> Div {
    let tags = |outline: bool| {
        row().children(
            [
                ("Default", Tag::new()),
                ("Primary", Tag::primary()),
                ("Secondary", Tag::secondary()),
                ("Danger", Tag::danger()),
                ("Success", Tag::success()),
                ("Warning", Tag::warning()),
                ("Info", Tag::info()),
            ]
            .map(|(name, tag)| tag.when(outline, |this| this.outline()).child(name)),
        )
    };

    let badges = row()
        .gap_6()
        .child(
            Badge::new()
                .count(3)
                .child(Avatar::new().name("Ada Lovelace")),
        )
        .child(
            Badge::new()
                .count(1200)
                .max(99)
                .child(Avatar::new().name("Grace Hopper")),
        )
        .child(
            Badge::new()
                .dot()
                .child(Icon::new(IconName::Settings).size_6()),
        )
        .child(
            Badge::new()
                .dot()
                .color(cx.theme().success)
                .child(Avatar::new().name("Alan Turing")),
        );

    let alerts = v_flex()
        .w_full()
        .gap_2()
        .child(Alert::info("alert-info", "Something worth knowing.").title("Info"))
        .child(Alert::success("alert-success", "That worked.").title("Success"))
        .child(Alert::warning("alert-warning", "This might not go well.").title("Warning"))
        .child(Alert::error("alert-error", "That didn't work.").title("Error"))
        .child(Alert::info("alert-plain", "An alert with no title."));

    let spinners = row()
        .gap_4()
        .children(SIZES.map(|(_, size)| Spinner::new().with_size(size)))
        .child(Spinner::new().color(cx.theme().primary));

    let progress = v_flex()
        .w(px(FIELD_WIDTH))
        .gap_3()
        .child(Progress::new().value(0.))
        .child(Progress::new().value(35.))
        .child(Progress::new().value(100.));

    let skeleton = h_flex()
        .gap_3()
        .items_start()
        .child(Skeleton::new().size_10().rounded_full())
        .child(
            v_flex()
                .gap_2()
                .child(Skeleton::new().w(px(120.)).h_4())
                .child(Skeleton::new().w(px(220.)).h_4())
                .child(Skeleton::new().secondary().w(px(180.)).h_4()),
        );

    let keys = row().children(
        ["ctrl-k", "shift-enter", "escape", "alt-up"]
            .into_iter()
            .filter_map(|key| Keystroke::parse(key).ok())
            .map(Kbd::new),
    );

    let dividers = v_flex()
        .w(px(FIELD_WIDTH * 1.5))
        .gap_4()
        .child(Divider::horizontal())
        .child(Divider::horizontal().label("Labelled"))
        .child(Divider::horizontal_dashed())
        .child(
            h_flex()
                .h_6()
                .gap_3()
                .child("Left")
                .child(Divider::vertical())
                .child("Right"),
        );

    stack()
        .child(specimen("Tag", tags(false), cx))
        .child(specimen("Tag, outline", tags(true), cx))
        .child(specimen("Badge", badges, cx))
        .child(specimen("Alert", alerts, cx))
        .child(specimen("Spinner", spinners, cx))
        .child(specimen("Progress", progress, cx))
        .child(specimen("Skeleton", skeleton, cx))
        .child(specimen("Kbd", keys, cx))
        .child(specimen("Divider", dividers, cx))
}

/// Applies one of the tab bar's visual variants.
type TabStyle = fn(TabBar) -> TabBar;

fn navigation(window: &mut Window, cx: &mut App) -> Div {
    let variants: [(&'static str, TabStyle); 5] = [
        ("Default", |bar| bar),
        ("Underline", TabBar::underline),
        ("Pill", TabBar::pill),
        ("Outline", TabBar::outline),
        ("Segmented", TabBar::segmented),
    ];

    let mut block = stack();
    for (name, variant) in variants {
        let selected = demo_state(name, window, cx, |_, _| 0usize);
        let bar = variant(TabBar::new(SharedString::from(format!("tabs-{name}"))))
            .selected_index(*selected.read(cx))
            .on_click(store(&selected))
            .child(Tab::new().label("Online"))
            .child(Tab::new().label("All"))
            .child(Tab::new().label("Pending"))
            .child(Tab::new().label("Blocked").disabled(true));
        block = block.child(specimen(name, bar, cx));
    }
    block
}

fn avatars(_: &mut Window, cx: &mut App) -> Div {
    let names = [
        "Ada Lovelace",
        "Grace Hopper",
        "Alan Turing",
        "Edsger Dijkstra",
    ];

    let sizes = row()
        .gap_4()
        .children(SIZES.map(|(_, size)| Avatar::new().name("Ada Lovelace").with_size(size)));

    let placeholder = row()
        .gap_4()
        .child(Avatar::new())
        .child(Avatar::new().placeholder(IconName::Settings));

    let group = AvatarGroup::new()
        .limit(3)
        .ellipsis()
        .children(names.map(|name| Avatar::new().name(name)));

    stack()
        .child(specimen("Sizes", sizes, cx))
        .child(specimen("Placeholder", placeholder, cx))
        .child(specimen("Group", group, cx))
}

/// Every icon the asset source can serve: gpui-component's bundled set plus the
/// ones the app ships itself.
fn icons(_: &mut Window, cx: &mut App) -> Div {
    let mut paths = cx.asset_source().list("icons/").unwrap_or_default();
    paths.sort();
    paths.dedup();

    let muted = cx.theme().muted_foreground;
    div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(paths.into_iter().map(|path| {
            let name = path
                .trim_start_matches("icons/")
                .trim_end_matches(".svg")
                .to_string();
            v_flex()
                .w(px(88.))
                .py_2()
                .items_center()
                .gap_1p5()
                .child(Icon::default().path(path).size_5())
                .child(
                    div()
                        .w_full()
                        .px_1()
                        .text_xs()
                        .text_center()
                        .truncate()
                        .text_color(muted)
                        .child(name),
                )
        }))
}
