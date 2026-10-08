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

    /// Stretches every control's target `distance` past its top edge, and
    /// close's past its right edge too, so flinging the pointer to the window's
    /// edge lands on a button rather than the gutter around the panel.
    pub fn reach(mut self, distance: Pixels) -> Self {
        self.reach = distance;
        self
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let control = |control: Control, window: &mut Window, cx: &mut App| {
            // Per strip, since moving between them reports enter and leave in
            // no particular order.
            let reach_hovered = window.use_keyed_state(control.reach_id(), cx, |_, _| [false; 2]);
            let highlight = reach_hovered.read(cx).contains(&true);
            let (hover_bg, hover_fg, _) = control.colors(cx);

            let element = control
                .element(cx)
                .when(highlight, |this| this.bg(hover_bg).text_color(hover_fg));
            if self.reach > px(0.) {
                element
                    .relative()
                    .child(reach(control, self.reach, reach_hovered))
            } else {
                element
            }
        };

        let middle = if window.is_maximized() {
            Control::Restore
        } else {
            Control::Maximize
        };

        h_flex()
            .id("window-controls")
            .h_full()
            .flex_shrink_0()
            .items_center()
            .child(control(Control::Minimize, window, cx))
            .child(control(middle, window, cx))
            .child(control(Control::Close, window, cx).rounded_tr(self.corner))
    }
}

/// Invisible strips extending a control (see [`WindowControls::reach`]).
///
/// Deferred so the panel's clipping doesn't cut them off, and occluding so the
/// window's drag strip, painted first, doesn't claim them.
fn reach(control: Control, distance: Pixels, hovered: Entity<[bool; 2]>) -> impl IntoElement {
    let strip = |index: usize| {
        let hovered = hovered.clone();
        div()
            .id((control.reach_id(), index))
            .absolute()
            .occlude()
            .on_hover(move |is_hovered, _window, cx| {
                hovered.update(cx, |hovered, cx| {
                    hovered[index] = *is_hovered;
                    cx.notify();
                })
            })
            .when(cfg!(target_os = "windows"), |this| {
                this.window_control_area(control.area())
            })
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_click(move |_event, window, _cx| control.act(window))
            })
    };

    deferred(
        div()
            .absolute()
            .inset_0()
            .child(
                strip(0)
                    .top(-distance)
                    .left_0()
                    .right(if control.is_close() {
                        -distance
                    } else {
                        px(0.)
                    })
                    .h(distance),
            )
            .when(control.is_close(), |this| {
                this.child(strip(1).top_0().bottom_0().right(-distance).w(distance))
            }),
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

    fn reach_id(self) -> &'static str {
        match self {
            Self::Minimize => "window-minimize-reach",
            Self::Maximize => "window-maximize-reach",
            Self::Restore => "window-restore-reach",
            Self::Close => "window-close-reach",
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

    /// Hover background and foreground, and pressed background.
    fn colors(self, cx: &App) -> (Hsla, Hsla, Hsla) {
        let theme = cx.theme();
        if self.is_close() {
            (theme.danger, theme.danger_foreground, theme.danger_active)
        } else {
            (
                theme.secondary_hover,
                theme.secondary_foreground,
                theme.secondary_active,
            )
        }
    }

    fn act(self, window: &mut Window) {
        match self {
            Self::Minimize => window.minimize_window(),
            Self::Maximize | Self::Restore => window.zoom_window(),
            Self::Close => window.remove_window(),
        }
    }
}

impl Control {
    fn element(self, cx: &App) -> Stateful<Div> {
        let (hover_bg, hover_fg, active_bg) = self.colors(cx);

        div()
            .id(self.id())
            .flex()
            .flex_shrink_0()
            .w(px(CONTROL_WIDTH))
            .h_full()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .hover(|style| style.bg(hover_bg).text_color(hover_fg))
            .active(|style| style.bg(active_bg).text_color(hover_fg))
            // Windows does the clicking itself: hit-testing hands these bounds
            // back as caption buttons, so the click never reaches the element.
            .when(cfg!(target_os = "windows"), |this| {
                this.window_control_area(self.area())
            })
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_click(move |_event, window, _cx| self.act(window))
            })
            .child(Icon::new(self.icon()).small())
    }
}
