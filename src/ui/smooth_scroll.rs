//! Smooth scrolling for scroll containers.
//!
//! Windows reports a wheel notch as one multi-line jump that GPUI applies in a
//! single frame. Its built-in handlers can't be stopped, so we let them move the
//! container, put it back, and ease toward the position they asked for.

use std::time::{Duration, Instant};

use gpui_kit::*;

/// Fraction of the remaining distance covered per frame. Each frame is a full
/// redraw, so this is a CPU knob too: 0.4 lands a notch in about nine frames.
const EASE: f32 = 0.4;
/// Below this, snap: sub-pixel frames cost a redraw and show nothing.
const SETTLE: f32 = 1.0;
/// Redraw ceiling for a glide. Under a 60Hz frame, so those displays step every
/// vsync while faster ones skip invisible frames.
const MIN_STEP: Duration = Duration::from_millis(15);
/// Line jumps up to this size land immediately.
///
/// A mouse notch is `wheel_scroll_lines` at once (three by default), which is
/// what needs easing. A precision touchpad uses the same Windows message but
/// reports fractions of a line often; it's already smooth and easing adds lag.
const DIRECT_LINES: f32 = 1.0;

/// Offsets follow GPUI: `0` is the top, negative scrolling down.
#[derive(Clone)]
enum Surface {
    Div(ScrollHandle),
    List(ListState),
    /// Kept whole for the list's `.track_scroll()`.
    UniformList(UniformListScrollHandle),
}

impl Surface {
    fn offset(&self) -> Pixels {
        match self {
            Self::Div(handle) => handle.offset().y,
            Self::UniformList(handle) => handle.0.borrow().base_handle.offset().y,
            // A bottom-aligned list parks its anchor one item past the last and this
            // reports the anchor's top: a viewport short of the drawn bottom.
            // Clamping folds that back.
            Self::List(state) => state
                .scroll_px_offset_for_scrollbar()
                .y
                .clamp(self.min_offset(), px(0.)),
        }
    }

    fn set_offset(&self, y: Pixels) {
        match self {
            Self::Div(handle) => handle.set_offset(point(handle.offset().x, y)),
            Self::UniformList(handle) => {
                let handle = &handle.0.borrow().base_handle;
                handle.set_offset(point(handle.offset().x, y));
            }
            // Repins a bottom-aligned list at the bottom, so new messages follow.
            Self::List(state) => state.set_offset_from_scrollbar(point(px(0.), y)),
        }
    }

    fn min_offset(&self) -> Pixels {
        match self {
            Self::Div(handle) => -handle.max_offset().y,
            Self::UniformList(handle) => -handle.0.borrow().base_handle.max_offset().y,
            // Must match the setter's clamp exactly, or a `min_offset` target stops
            // short and the list never repins. That holds only while the list has
            // no padding: the setter counts it in the range and this doesn't, so
            // `py_2` would leave us 16px short.
            Self::List(state) => -state.max_offset_for_scrollbar().y,
        }
    }
}

/// Drive with [`step`](Self::step) once per render and
/// [`absorb`](Self::absorb) from the container's `on_scroll_wheel`.
pub struct SmoothScroll {
    surface: Surface,
    current: Pixels,
    target: Pixels,
}

impl SmoothScroll {
    pub fn div() -> Self {
        Self::new(Surface::Div(ScrollHandle::new()))
    }

    pub fn list(state: ListState) -> Self {
        Self::new(Surface::List(state))
    }

    pub fn uniform_list() -> Self {
        Self::new(Surface::UniformList(UniformListScrollHandle::new()))
    }

    fn new(surface: Surface) -> Self {
        Self {
            surface,
            current: px(0.),
            target: px(0.),
        }
    }

    /// Div surfaces only.
    pub fn handle(&self) -> &ScrollHandle {
        match &self.surface {
            Surface::Div(handle) => handle,
            Surface::List(_) => unreachable!("list surfaces are tracked by their ListState"),
            Surface::UniformList(_) => unreachable!("see `uniform_handle`"),
        }
    }

    /// Uniform list surfaces only.
    pub fn uniform_handle(&self) -> &UniformListScrollHandle {
        match &self.surface {
            Surface::UniformList(handle) => handle,
            _ => unreachable!("only uniform lists track a UniformListScrollHandle"),
        }
    }

    pub fn offset(&self) -> Pixels {
        self.current
    }

    /// Eases toward `y` as a wheel notch would. The caller notifies the view.
    pub fn glide_to(&mut self, y: Pixels) {
        self.target = y.clamp(self.surface.min_offset(), px(0.));
    }

    /// Moves to `y` at once, ending any glide; for replaced content.
    pub fn jump_to(&mut self, y: Pixels) {
        self.current = y.clamp(self.surface.min_offset().min(px(0.)), px(0.));
        self.target = self.current;
        self.surface.set_offset(self.current);
    }

    /// Advances the glide one frame. Call once per render, before building the element.
    pub fn step(&mut self, window: &Window) {
        self.sync();
        if self.current == self.target {
            return;
        }

        self.current = advance(self.current, self.target);
        self.surface.set_offset(self.current);
        request_step(window.current_view(), Instant::now() + MIN_STEP, window);
    }

    /// Catches up on moves the container made on its own: older messages spliced
    /// in above, items re-measured in layout. Call from a capture-phase wheel
    /// listener too, since those land between renders and [`absorb`](Self::absorb)
    /// would otherwise add its delta to a stale offset, jumping by the drift.
    pub fn sync(&mut self) {
        let offset = self.surface.offset();
        let shift = offset - self.current;
        if shift == px(0.) {
            return;
        }
        // Mid-glide, carry the target with the shift; steering to the raw number
        // would slam the list to the top when history lands.
        if self.current != self.target {
            self.target = (self.target + shift).clamp(self.surface.min_offset(), px(0.));
        } else {
            self.target = offset;
        }
        self.current = offset;
    }

    /// Overwrites the jump the container's own handler just applied and folds the
    /// event's delta into the target. Call from `on_scroll_wheel` on the container
    /// (for a list, an ancestor), which bubbles after that handler.
    ///
    /// The delta comes off the event, not from how far the container moved: a list
    /// re-applies deltas since the last paint as one coalesced total, and
    /// coalescing discards the running total when a delta flips sign, so one stray
    /// opposite tick (routine on touchpads) would measure as the whole gesture
    /// reversed.
    ///
    /// Requests no redraw; the handler we overwrite already notified the view.
    pub fn absorb(&mut self, event: &ScrollWheelEvent, window: &Window) {
        let (delta, smooth) = wheel_delta(&event.delta, window.line_height());
        if delta.is_zero() {
            return;
        }

        self.target = (self.target + delta).clamp(self.surface.min_offset(), px(0.));
        if !smooth {
            self.current = self.target;
        }
        // Nothing painted since the container moved, so its jump is never seen.
        self.surface.set_offset(self.current);
    }
}

fn wheel_delta(delta: &ScrollDelta, line_height: Pixels) -> (Pixels, bool) {
    match delta {
        ScrollDelta::Pixels(delta) => (delta.y, false),
        ScrollDelta::Lines(delta) => (line_height * delta.y, delta.y.abs() > DIRECT_LINES),
    }
}

/// Snapping the last step guarantees the glide ends; crawling at float
/// precision would redraw forever.
fn advance(current: Pixels, target: Pixels) -> Pixels {
    let remaining = target - current;
    if remaining.abs() < px(SETTLE) {
        target
    } else {
        current + remaining * EASE
    }
}

/// Requests a redraw once `due` has passed, riding the vsync callback until then.
fn request_step(view: EntityId, due: Instant, window: &Window) {
    window.on_next_frame(move |window, cx| {
        if Instant::now() >= due {
            cx.notify(view);
        } else {
            request_step(view, due, window);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{advance, wheel_delta};
    use gpui_kit::{ScrollDelta, point, px};

    #[test]
    fn a_glide_spreads_the_distance_then_lands_exactly() {
        let target = px(-137.5);
        let mut current = px(0.);
        let mut frames = 0;
        while current != target {
            let next = advance(current, target);
            assert!(next != current, "stalled at {current:?}");
            current = next;
            frames += 1;
            assert!(frames < 60, "never settled, so the redraws never stop");
        }
        assert!(
            frames > 3,
            "landed in {frames} frame(s), no smoothing happened"
        );
    }

    #[test]
    fn a_notch_eases_and_a_touchpad_tick_does_not() {
        let line_height = px(20.);

        assert_eq!(
            wheel_delta(&ScrollDelta::Lines(point(0., -3.)), line_height),
            (px(-60.), true)
        );

        let (delta, smooth) = wheel_delta(&ScrollDelta::Lines(point(0., -0.2)), line_height);
        assert_eq!(delta, px(-4.));
        assert!(!smooth, "easing a touchpad's own stream only adds lag");

        assert_eq!(
            wheel_delta(&ScrollDelta::Pixels(point(px(0.), px(-7.5))), line_height),
            (px(-7.5), false)
        );
    }
}
