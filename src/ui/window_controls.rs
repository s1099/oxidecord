//! The window's minimize, maximize and close buttons.
//!
//! The window has no system title bar (see `main`). [`WindowControls`] sits at the
//! right end of an existing header, whose free space drags the window (see
//! [`crate::screens::home::view::content::header`]).

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Matches the Windows shell's caption buttons.
const CONTROL_WIDTH: f32 = 46.;

#[derive(IntoElement, Default)]
pub struct WindowControls {
    corner: Pixels,
    reach: Pixels,
}

impl WindowControls {
    /// Rounds close's outer corner; gpui clips to rectangles, so its hover fill
    /// would otherwise square off a rounded panel's corner.
    pub fn corner(mut self, radius: Pixels) -> Self {
        self.corner = radius;
        self
    }

    /// Extends every control's hit area `distance` past its top edge (close also
    /// past its right), so flinging the pointer to the window edge hits a button.
    pub fn reach(mut self, distance: Pixels) -> Self {
        self.reach = distance;
        self
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let control = |control: Control, window: &mut Window, cx: &mut App| {
            // Per strip: moving between them reports enter and leave in no fixed order.
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
/// Deferred to escape the panel's clipping; occluding so the drag strip, painted
/// first, doesn't claim them.
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
            // Hit-testing reports these bounds as caption buttons, so Windows handles
            // the click and it never reaches the element.
            .when(cfg!(target_os = "windows"), |this| {
                this.window_control_area(self.area())
            })
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_click(move |_event, window, _cx| self.act(window))
            })
            .child(Icon::new(self.icon()).small())
    }
}
