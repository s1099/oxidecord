//! Depth: the UI is lit from above. Clickable things sit *on* the page, floating
//! layers hover *over* it, everything else (inputs included) is flat.
//!
//! Each [`Level`] is a lit top edge, sometimes a shaded bottom edge, and drop
//! shadows that grow with elevation, all kept faint. Small controls get the most
//! and big surfaces the least, since on a large area a sheen reads as a smudge.
//!
//! gpui has no inset shadows and its gradients take two stops, so edges and
//! sheens are absolutely positioned layers over the fill. [`Lit::lit`] wires
//! both up.
//!
//! Black and white at low alpha rather than theme colours, so the light reads
//! the same on every preset. Dark themes get stronger shadows and fainter highlights.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Outline and secondary buttons, chips, active rows.
    Raised,
    /// The primary button, tooltips.
    RaisedStrong,
    /// A button held down, or a toggle that's on.
    Pressed,
    /// Cards and the content pane.
    Surface,
    /// Dialogs, menus, popovers.
    Overlay,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// Big surfaces always use this.
    Matte,
    /// Fades out by 70%; for small controls.
    Subtle,
}

pub const PRESS_GROUP: &str = "depth-press";

/// Lit edge fade, in pixels. A hard 1px line can't follow a rounded corner, so
/// the edge is a gradient over the whole element, clipped by its rounding.
const EDGE: f32 = 2.;

const INNER_SHADOW: f32 = 3.;

pub fn shadow(level: Level, cx: &App) -> Vec<BoxShadow> {
    let dark = cx.theme().mode.is_dark();
    let drop = |y: f32, blur: f32, spread: f32, alpha: f32| BoxShadow {
        color: black_a(alpha),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    };

    match (level, dark) {
        (Level::Raised, false) => vec![drop(1., 2., 0., 0.035)],
        (Level::Raised, true) => vec![drop(1., 2., 0., 0.18)],
        (Level::RaisedStrong, false) => vec![drop(1., 2., 0., 0.07)],
        (Level::RaisedStrong, true) => vec![drop(1., 2., 0., 0.25)],
        (Level::Pressed, _) => vec![],
        (Level::Surface, false) => vec![drop(1., 2., 0., 0.03)],
        (Level::Surface, true) => vec![drop(1., 2., 0., 0.15)],
        (Level::Overlay, false) => vec![drop(2., 6., -2., 0.06), drop(12., 32., -8., 0.14)],
        (Level::Overlay, true) => vec![drop(2., 8., -2., 0.3), drop(16., 40., -12., 0.5)],
    }
}

/// Painted layers for an element about `height` tall. Put them first among the
/// children of a `relative` element so content paints over them.
pub fn layers(level: Level, finish: Finish, height: Pixels, radius: Pixels, cx: &App) -> Vec<Div> {
    let dark = cx.theme().mode.is_dark();
    let mut layers = Vec::new();

    match finish {
        Finish::Matte => {}
        Finish::Subtle => {
            let alpha = if dark { 0.02 } else { 0.25 };
            layers.push(top_fade(white_a(alpha), 0.7, radius));
        }
    }

    let (highlight, shade) = match (level, dark) {
        (Level::Raised, false) => (0.35, 0.),
        (Level::Raised, true) => (0.035, 0.),
        (Level::RaisedStrong, false) => (0.07, 0.05),
        (Level::RaisedStrong, true) => (0.14, 0.05),
        (Level::Overlay, false) => (0.5, 0.),
        (Level::Overlay, true) => (0.05, 0.),
        _ => (0., 0.),
    };
    let edge = (EDGE / f32::from(height)).min(0.5);
    if highlight > 0. {
        layers.push(top_fade(white_a(highlight), edge, radius));
    }
    if shade > 0. {
        layers.push(bottom_fade(black_a(shade), 1. - edge, radius));
    }

    if level == Level::Pressed {
        layers.push(pressed(height, radius, cx));
    }

    layers
}

/// Separate layer so it shows only while held (see [`PRESS_GROUP`]).
pub fn pressed(height: Pixels, radius: Pixels, cx: &App) -> Div {
    let alpha = if cx.theme().mode.is_dark() { 0.2 } else { 0.06 };
    let reach = (INNER_SHADOW / f32::from(height)).min(0.5);
    top_fade(black_a(alpha), reach, radius)
}

fn top_fade(color: Hsla, until: f32, radius: Pixels) -> Div {
    layer(radius).bg(linear_gradient(
        180.,
        linear_color_stop(color, 0.),
        linear_color_stop(color.alpha(0.), until),
    ))
}

fn bottom_fade(color: Hsla, from: f32, radius: Pixels) -> Div {
    layer(radius).bg(linear_gradient(
        180.,
        linear_color_stop(color.alpha(0.), from),
        linear_color_stop(color, 1.),
    ))
}

fn layer(radius: Pixels) -> Div {
    div().absolute().inset_0().rounded(radius)
}

fn black_a(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}

fn white_a(alpha: f32) -> Hsla {
    hsla(0., 0., 1., alpha)
}

pub trait Lit: Styled + ParentElement + Sized {
    /// Call before adding content so it paints over the layers. `height` only
    /// needs to be roughly right.
    fn lit(self, level: Level, finish: Finish, height: Pixels, radius: Pixels, cx: &App) -> Self {
        self.relative()
            .rounded(radius)
            .shadow(shadow(level, cx))
            .children(layers(level, finish, height, radius, cx))
    }
}

impl<T: Styled + ParentElement> Lit for T {}

/// Shown only while an element in [`PRESS_GROUP`] is held. Needs its own id for
/// gpui to track the press.
pub fn press_layer(height: Pixels, radius: Pixels, cx: &App) -> Stateful<Div> {
    pressed(height, radius, cx)
        .id("press-layer")
        .invisible()
        .group_active(PRESS_GROUP, |this| this.visible())
}

/// Hairline outline for cards and overlays: foreground at 10%.
pub fn ring(cx: &App) -> Hsla {
    cx.theme().foreground.opacity(0.1)
}

/// Shadow under an image, video card or embed: a tight contact shadow for the
/// edge plus a wider, softer one for lift, as in Discord's elevation tokens.
pub fn media_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: black_a(0.16),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: black_a(0.12),
            offset: point(px(0.), px(2.)),
            blur_radius: px(5.),
            spread_radius: px(-2.),
            inset: false,
        },
    ]
}

/// A surface with a hairline ring and no sheen. Like [`Lit::lit`], call before
/// adding content.
pub fn card(radius: Pixels, cx: &App) -> Div {
    div()
        .bg(cx.theme().background)
        .border_1()
        .border_color(ring(cx))
        .lit(Level::Surface, Finish::Matte, px(200.), radius - px(1.), cx)
        .rounded(radius)
}

pub mod radius {
    use gpui_kit::{Pixels, px};

    pub const CONTROL: Pixels = px(10.);
    pub const ITEM: Pixels = px(8.);
    pub const CARD: Pixels = px(14.);
}
