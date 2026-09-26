//! Depth: the UI is lit from above. Things you click sit *on* the page and
//! floating layers hover *over* it; everything else, inputs included, is flat.
//!
//! Each [`Level`] is built from the same ingredients: a lit top edge, sometimes
//! a shaded bottom edge, and drop shadows that grow with elevation. All of it
//! is kept faint — enough to tell a control from a label, not so much that the
//! UI looks embossed. Small controls get the most and big surfaces the least,
//! since on a large area a sheen reads as a smudge and a shadow as a heavy box.
//!
//! gpui has no inset shadows and its gradients take two stops, so the edges and
//! sheens are painted as absolutely positioned layers over the element's fill
//! rather than as part of its shadow. [`Lit::lit`] wires both up at once.
//!
//! Plain black and white at low alpha throughout, rather than theme colours, so
//! the light and shade read the same on every preset. Dark themes get stronger
//! shadows and much fainter highlights, which is how light actually falls on a
//! dark surface.

use gpui::*;
use gpui_component::ActiveTheme as _;

/// How far above or into the page something sits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Sits on the page: outline and secondary buttons, chips, active rows.
    Raised,
    /// A filled, prominent control: the primary button, tooltips.
    RaisedStrong,
    /// Pushed in: a button held down, or a toggle that's on.
    Pressed,
    /// A large resting panel: cards and the content pane.
    Surface,
    /// Floats above everything: dialogs, menus, popovers.
    Overlay,
}

/// The sheen over a fill, on top of its level's edges.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// No sheen. Big surfaces always use this.
    Matte,
    /// A faint highlight fading out by 70%: small controls.
    Subtle,
}

/// Group name the lighting layers use to follow a control's pressed state.
pub const PRESS_GROUP: &str = "depth-press";

/// How far a lit edge fades in, in pixels. A hard 1px line can't follow a
/// rounded corner, so the edge is a short gradient over the whole element,
/// which the element's own rounding clips.
const EDGE: f32 = 2.;

/// How far a pressed element's inner shadow reaches.
const INNER_SHADOW: f32 = 3.;

pub fn shadow(level: Level, cx: &App) -> Vec<BoxShadow> {
    let dark = cx.theme().mode.is_dark();
    let drop = |y: f32, blur: f32, spread: f32, alpha: f32| BoxShadow {
        color: black_a(alpha),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    };

    match (level, dark) {
        (Level::Raised, false) => vec![drop(1., 2., 0., 0.05)],
        (Level::Raised, true) => vec![drop(1., 2., 0., 0.25)],
        (Level::RaisedStrong, false) => vec![drop(1., 2., 0., 0.1)],
        (Level::RaisedStrong, true) => vec![drop(1., 2., 0., 0.35)],
        (Level::Pressed, _) => vec![],
        (Level::Surface, false) => vec![drop(1., 2., 0., 0.03)],
        (Level::Surface, true) => vec![drop(1., 2., 0., 0.15)],
        (Level::Overlay, false) => vec![drop(2., 6., -2., 0.06), drop(12., 32., -8., 0.14)],
        (Level::Overlay, true) => vec![drop(2., 8., -2., 0.3), drop(16., 40., -12., 0.5)],
    }
}

/// The painted layers for `level` and `finish`, for an element about `height`
/// tall with corners of `radius`. Goes first among the element's children,
/// which must be positioned `relative`, so its content paints over them.
pub fn layers(level: Level, finish: Finish, height: Pixels, radius: Pixels, cx: &App) -> Vec<Div> {
    let dark = cx.theme().mode.is_dark();
    let mut layers = Vec::new();

    match finish {
        Finish::Matte => {}
        Finish::Subtle => {
            let alpha = if dark { 0.03 } else { 0.35 };
            layers.push(top_fade(white_a(alpha), 0.7, radius));
        }
    }

    let (highlight, shade) = match (level, dark) {
        (Level::Raised, false) => (0.5, 0.),
        (Level::Raised, true) => (0.05, 0.),
        (Level::RaisedStrong, false) => (0.1, 0.08),
        (Level::RaisedStrong, true) => (0.2, 0.08),
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

/// The inner shadow of a pushed-in control, as a layer of its own so it can be
/// shown only while the control is held (see [`PRESS_GROUP`]).
pub fn pressed(height: Pixels, radius: Pixels, cx: &App) -> Div {
    let alpha = if cx.theme().mode.is_dark() {
        0.25
    } else {
        0.08
    };
    let reach = (INNER_SHADOW / f32::from(height)).min(0.5);
    top_fade(black_a(alpha), reach, radius)
}

/// A layer shading from `color` at the top to nothing at `until` (a fraction of
/// the height).
fn top_fade(color: Hsla, until: f32, radius: Pixels) -> Div {
    layer(radius).bg(linear_gradient(
        180.,
        linear_color_stop(color, 0.),
        linear_color_stop(color.alpha(0.), until),
    ))
}

/// A layer shading from nothing at `from` (a fraction of the height) to `color`
/// at the bottom.
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

/// Gives any element a depth level: its drop shadows, its painted edges and
/// sheen, and the rounding they're clipped to.
pub trait Lit: Styled + ParentElement + Sized {
    /// Call before adding the element's content, so the content paints over
    /// the layers. `height` only needs to be roughly right — it sets how far
    /// the edges fade in.
    fn lit(self, level: Level, finish: Finish, height: Pixels, radius: Pixels, cx: &App) -> Self {
        self.relative()
            .rounded(radius)
            .shadow(shadow(level, cx))
            .children(layers(level, finish, height, radius, cx))
    }
}

impl<T: Styled + ParentElement> Lit for T {}

/// The pressed layer, shown only while an element in [`PRESS_GROUP`] is held
/// down. It needs an id of its own for gpui to track the press.
pub fn press_layer(height: Pixels, radius: Pixels, cx: &App) -> Stateful<Div> {
    pressed(height, radius, cx)
        .id("press-layer")
        .invisible()
        .group_active(PRESS_GROUP, |this| this.visible())
}

/// The fine outline cards and overlays draw around themselves: the foreground
/// at a tenth, so it's a hairline on any theme.
pub fn ring(cx: &App) -> Hsla {
    cx.theme().foreground.opacity(0.1)
}

/// The shadow under an image, a video card, or an embed.
///
/// Two layers, the way Discord's own elevation tokens are built: a tight
/// contact shadow that draws the edge against the background, and a wider,
/// softer one that does the lifting.
pub fn media_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: black_a(0.16),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: black_a(0.12),
            offset: point(px(0.), px(2.)),
            blur_radius: px(5.),
            spread_radius: px(-2.),
        },
    ]
}

/// A card: the page colour lifted onto a surface with a hairline ring. No sheen
/// — see the module docs. Like [`Lit::lit`], call before adding content.
pub fn card(radius: Pixels, cx: &App) -> Div {
    div()
        .bg(cx.theme().background)
        .border_1()
        .border_color(ring(cx))
        .lit(Level::Surface, Finish::Matte, px(200.), radius - px(1.), cx)
        .rounded(radius)
}

/// Corner radii, one per kind of thing, so everything of a kind matches.
pub mod radius {
    use gpui::{Pixels, px};

    /// Buttons, inputs, and other controls at their default size.
    pub const CONTROL: Pixels = px(10.);
    /// Small controls, menu items, and sidebar rows.
    pub const ITEM: Pixels = px(8.);
    /// Cards, dialogs, and the content pane.
    pub const CARD: Pixels = px(14.);
}
