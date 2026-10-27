//! The resizable column beside the server rail: a guild's channels, or the DM list.

mod channel_list;
mod channels;
mod dms;

use std::rc::Rc;

use gpui_kit::base::{
    HandleEdge, InteractiveElementExt as _, ResizeHandleContext, ResizeHandleState, Transition,
    transition,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, resizable::resize_handle, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform::prefs;
use crate::screens::home::HomeScreen;

const DEFAULT_WIDTH: f32 = 240.;
const MIN_WIDTH: f32 = 180.;
const MAX_WIDTH: f32 = 420.;

/// The width to open at, from the saved one.
pub(in crate::screens::home) fn sidebar_width(saved: Option<f32>) -> Pixels {
    px(saved.unwrap_or(DEFAULT_WIDTH).clamp(MIN_WIDTH, MAX_WIDTH))
}

/// The drag payload of the sidebar's resize handle.
#[derive(Clone)]
pub(in crate::screens::home) struct ResizeSidebar;

impl Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl HomeScreen {
    /// Sizes either sidebar and puts the drag handle on its trailing edge.
    pub(super) fn render_sidebar_frame(
        &self,
        sidebar: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .relative()
            .h_full()
            .w(self.sidebar_width)
            .flex_shrink_0()
            .child(sidebar)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ResizeSidebar>, _, cx| {
                    let width = (event.event.position.x - event.bounds.left())
                        .clamp(px(MIN_WIDTH), px(MAX_WIDTH));
                    if width != this.sidebar_width {
                        this.sidebar_width = width;
                        cx.notify();
                    }
                }),
            )
            .child(
                resize_handle("sidebar-resize", Axis::Horizontal)
                    .with_appearance(Rc::new(|handle, window, cx| {
                        Some(render_handle(handle, window, cx))
                    }))
                    .inside(HandleEdge::Trailing)
                    .on_drag(ResizeSidebar, |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .on_double_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_width = px(DEFAULT_WIDTH);
                        prefs::update(|prefs| prefs.sidebar_width = None);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(in crate::screens::home) fn on_sidebar_resized(
        &mut self,
        _: &ResizeSidebar,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        let width = f32::from(self.sidebar_width);
        prefs::update(|prefs| prefs.sidebar_width = Some(width));
    }
}

/// gpui-kit's divider without its hairline: the sidebar has no border, and the
/// conversation panel's ring already draws the edge.
fn render_handle(handle: &ResizeHandleContext, window: &mut Window, cx: &mut App) -> AnyElement {
    let (target_length, target_opacity) = match handle.state() {
        ResizeHandleState::Idle => (px(0.), 0.),
        ResizeHandleState::Hovered => (px(20.), 0.35),
        ResizeHandleState::Pressed => (px(28.), 0.6),
        ResizeHandleState::Dragging => (px(44.), 0.9),
    };
    let motion = cx.theme().motion_tokens();
    let policy = Transition::new(motion.duration_fast).easing(motion.easing_move.clone());
    // Sampled every frame, even at rest, so the next hover starts from where the last ended.
    let length = transition(
        "sidebar-handle-length",
        target_length,
        policy.clone(),
        window,
        cx,
    );
    let opacity =
        transition("sidebar-handle-opacity", target_opacity, policy, window, cx).clamp(0., 1.);

    div()
        .flex_none()
        .flex()
        .w(px(1.))
        .h_full()
        .items_center()
        .when(length > px(0.5), |line| {
            // Deferred so the conversation panel, painted after, doesn't cover its outer half.
            line.child(deferred(
                div()
                    .flex_none()
                    .w(px(3.))
                    .h(length)
                    .ml(px(-1.))
                    .rounded(cx.theme().radius_full())
                    .bg(cx.theme().muted_foreground)
                    .opacity(opacity),
            ))
        })
        .into_any_element()
}

/// The chrome both sidebars share: a titled header over the scrolling list.
/// The account panel is drawn by the home screen under both, since it spans the rail too.
fn shell(title: impl Into<SharedString>, list: impl IntoElement, cx: &App) -> impl IntoElement {
    let theme = cx.theme();

    // No borders: the sidebar is the window itself, and the conversation panel
    // beside it carries the edge.
    v_flex()
        .size_full()
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
