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
#[derive(IntoElement)]
pub struct WindowControls;

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .id("window-controls")
            .h_full()
            .flex_shrink_0()
            .items_center()
            .child(Control::Minimize)
            // The middle button swaps with the window's state, the way every
            // other window on the desktop does.
            .child(if window.is_maximized() {
                Control::Restore
            } else {
                Control::Maximize
            })
            .child(Control::Close)
    }
}

#[derive(IntoElement, Clone, Copy)]
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

impl RenderOnce for Control {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
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
