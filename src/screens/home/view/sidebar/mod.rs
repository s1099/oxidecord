//! The 240px column beside the server rail: a guild's channels, or the DM list.

mod channel_list;
mod channels;
mod dms;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

/// The chrome both sidebars share: a titled header over the scrolling list.
/// The account panel is drawn by the home screen under both, since it spans the rail too.
fn shell(title: impl Into<SharedString>, list: impl IntoElement, cx: &App) -> impl IntoElement {
    let theme = cx.theme();

    // No borders: the sidebar is the window itself, and the conversation panel
    // beside it carries the edge.
    v_flex()
        .w(px(240.))
        .h_full()
        .flex_shrink_0()
        .bg(theme.sidebar)
        .text_color(theme.sidebar_foreground)
        .child(
            h_flex()
                // Level with the conversation header, which sits inside the
                // inset panel.
                .h(px(56.))
                .pt_2()
                .flex_shrink_0()
                .px_4()
                .items_center()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                ),
        )
        .child(list)
}
