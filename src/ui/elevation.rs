//! Drop shadows that lift inline media off the message list.

use gpui::{BoxShadow, Hsla, hsla, point, px};

/// The shadow under an image, a video card, or an embed.
///
/// Two layers, the way Discord's own elevation tokens are built: a tight
/// contact shadow that draws the edge against the background, and a wider,
/// softer one that does the lifting. One layer on its own picks a side — a
/// tight shadow alone barely reads, a soft one alone leaves the media looking
/// unanchored.
///
/// Plain black rather than a theme colour, because it reads as shadow in both
/// light and dark themes; a tinted one only reads as shadow against the theme
/// it was picked for.
pub fn media_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: shadow_color(0.16),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: shadow_color(0.12),
            offset: point(px(0.), px(2.)),
            blur_radius: px(5.),
            spread_radius: px(-2.),
        },
    ]
}

fn shadow_color(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}
