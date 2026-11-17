//! The app's scrollbar: a slim gutter beside the content rather than an
//! overlay, as Discord's chat has.

use gpui_kit::component::{
    ActiveTheme as _,
    scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode},
};
use gpui_kit::*;

/// Thinner than Discord's 16px chat scroller: a 12px gutter holding a 6px track and thumb.
pub const GUTTER: f32 = 12.;
const INSET: f32 = 3.;
const THUMB_MIN: f32 = 40.;

/// The gutter is the content's own space, so hovered rows stop short of it, and
/// the track shows even with nothing to scroll. `id` tells apart scrollbars
/// drawn from the same place.
pub fn gutter<H: ScrollbarHandle + Clone>(id: impl Into<ElementId>, handle: &H, cx: &App) -> Div {
    let theme = cx.theme();
    let thumb = theme.tokens.scrollbar_thumb;
    div()
        .relative()
        .flex_shrink_0()
        .w(px(GUTTER))
        .h_full()
        .child(
            div()
                .absolute()
                .inset(px(INSET))
                .rounded_full()
                .bg(theme.scrollbar_thumb.opacity(0.3)),
        )
        .child(
            Scrollbar::vertical(handle)
                .id(id)
                .viewport_from_layout()
                .mode(ScrollbarMode::Always)
                // Set on the base style so hover and drag keep it: Discord's thumb neither grows
                // nor recolors.
                .styles(|styles| {
                    styles
                        .track(|track| {
                            track
                                .width(px(GUTTER))
                                .bg(transparent_black())
                                .border_color(transparent_black())
                        })
                        .thumb(|style| {
                            style
                                .bg(thumb)
                                .width(px(GUTTER - INSET * 2.))
                                .inset(px(INSET))
                                .radius(px(GUTTER))
                                .min_length(px(THUMB_MIN))
                        })
                }),
        )
}
