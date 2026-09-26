//! The window's minimize, maximize and close buttons.
//!
//! The window is opened without a system title bar (see `main`), so these are
//! the only way left to control it. They aren't given a bar of their own:
//! [`WindowControls`] is dropped into the right end of a header that's already
//! there, and the space beside it drags the window (see
//! [`crate::screens::home::view::content::header`]).

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};

/// Width of a single control. Matches the Windows shell's own caption buttons,
/// so the cluster reads as a title bar even though it sits in the app's header.
const CONTROL_WIDTH: f32 = 46.;

/// The three caption buttons, in the platform's order.
#[derive(IntoElement, Default)]
pub struct WindowControls {
    corner: Pixels,
    reach: Pixels,
}

impl WindowControls {
    /// Rounds the close button's outer corner, for controls sitting in the
    /// corner of a rounded panel — gpui clips to rectangles, so the hover fill
    /// would otherwise square the panel's corner off.
    pub fn corner(mut self, radius: Pixels) -> Self {
        self.corner = radius;
        self
    }

    /// Stretches the close button's target `distance` past its top and right
    /// edges, for controls inset from the window's corner. Flinging the pointer
    /// into the corner should land on close, the way it does on every other
    /// window, rather than on the gutter around the panel.
    pub fn reach(mut self, distance: Pixels) -> Self {
        self.reach = distance;
        self
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Which of the two reach strips (above, beside) the pointer is over,
        // kept apart because moving between them reports one entering and the
        // other leaving in no particular order.
        let reach_hovered = window.use_keyed_state("window-close-reach", cx, |_, _| [false; 2]);
        let highlight = reach_hovered.read(cx).contains(&true);

        let mut close =
            Control::Close
                .element(cx)
                .rounded_tr(self.corner)
                .when(highlight, |this| {
                    let theme = cx.theme();
                    this.bg(theme.danger).text_color(theme.danger_foreground)
                });
        if self.reach > px(0.) {
            close = close
                .relative()
                .child(close_reach(self.reach, reach_hovered));
        }

        h_flex()
            .id("window-controls")
            .h_full()
            .flex_shrink_0()
            .items_center()
            .child(Control::Minimize.element(cx))
            // The middle button swaps with the window's state, the way every
            // other window on the desktop does.
            .child(
                if window.is_maximized() {
                    Control::Restore
                } else {
                    Control::Maximize
                }
                .element(cx),
            )
            .child(close)
    }
}

/// Invisible strips above and to the right of the close button that Windows
/// treats as part of it (see [`WindowControls::reach`]).
///
/// Deferred, because the button sits in a panel that clips to its bounds and
/// would clip these away with it. They also block the mouse: control areas
/// resolve in paint order, and without that the window's drag strip along the
/// top edge, painted first, would claim the corner.
fn close_reach(distance: Pixels, hovered: Entity<[bool; 2]>) -> impl IntoElement {
    let strip = |index: usize| {
        let hovered = hovered.clone();
        div()
            .id(("window-close-reach", index))
            .absolute()
            .occlude()
            .on_hover(move |is_hovered, _window, cx| {
                hovered.update(cx, |hovered, cx| {
                    hovered[index] = *is_hovered;
                    cx.notify();
                })
            })
            .when(cfg!(target_os = "windows"), |this| {
                this.window_control_area(WindowControlArea::Close)
            })
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_click(|_event, window, _cx| window.remove_window())
            })
    };

    deferred(
        div()
            .absolute()
            .inset_0()
            // Above, running on to the corner.
            .child(
                strip(0)
                    .top(-distance)
                    .left_0()
                    .right(-distance)
                    .h(distance),
            )
            // Beside, down to the button's bottom edge.
            .child(strip(1).top_0().bottom_0().right(-distance).w(distance)),
    )
}

#[derive(Clone, Copy)]
enum Control {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Control {
    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "window-minimize",
            Self::Maximize => "window-maximize",
            Self::Restore => "window-restore",
            Self::Close => "window-close",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Minimize => IconName::WindowMinimize,
            Self::Maximize => IconName::WindowMaximize,
            Self::Restore => IconName::WindowRestore,
            Self::Close => IconName::WindowClose,
        }
    }

    /// The caption button Windows should treat these bounds as.
    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    fn is_close(self) -> bool {
        matches!(self, Self::Close)
    }
}

impl Control {
    fn element(self, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        // Closing is the destructive one, and gets the red hover every desktop
        // uses for it; the other two stay in the neutral palette.
        let (hover_bg, hover_fg, active_bg) = if self.is_close() {
            (theme.danger, theme.danger_foreground, theme.danger_active)
        } else {
            (
                theme.secondary_hover,
                theme.secondary_foreground,
                theme.secondary_active,
            )
        };

        div()
            .id(self.id())
            .flex()
            .flex_shrink_0()
            .w(px(CONTROL_WIDTH))
            .h_full()
            .items_center()
            .justify_center()
            .text_color(theme.muted_foreground)
            .hover(|style| style.bg(hover_bg).text_color(hover_fg))
            .active(|style| style.bg(active_bg).text_color(hover_fg))
            // Windows does the clicking itself: hit-testing hands these bounds
            // back as caption buttons, so the click never reaches the element.
            .when(cfg!(target_os = "windows"), |this| {
                this.window_control_area(self.area())
            })
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_click(move |_event, window, _cx| match self {
                    Self::Minimize => window.minimize_window(),
                    Self::Maximize | Self::Restore => window.zoom_window(),
                    Self::Close => window.remove_window(),
                })
            })
            .child(Icon::new(self.icon()).small())
    }
}
