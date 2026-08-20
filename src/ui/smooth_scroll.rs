//! Browser-like smooth scrolling for scroll containers.
//!
//! Windows reports one wheel notch as a single multi-line jump, which GPUI's
//! containers apply in one frame — that's the line-to-line snapping. Their
//! built-in handlers can't be stopped, so instead we let them move the
//! container, put it straight back, and ease toward the position they asked for
//! over the following frames.

use std::time::{Duration, Instant};

use gpui::*;

/// Fraction of the distance still owed handed over each frame. Every frame of a
/// glide costs a full redraw, so this is a CPU knob as much as a feel one: 0.4
/// lands a notch in about nine frames.
const EASE: f32 = 0.4;
/// Below this, snap. Sub-pixel frames cost a redraw and show nothing for it.
const SETTLE: f32 = 1.0;
/// Redraw ceiling for a glide. Inside a 60Hz frame, so those displays step every
/// vsync as before while faster ones stop paying for invisible extra frames.
const MIN_STEP: Duration = Duration::from_millis(15);
/// Line jumps up to this size land immediately instead of being eased.
///
/// A mouse notch is `wheel_scroll_lines` at once — three by default — and that
/// jump is the whole reason this module exists. A precision touchpad arrives
/// through the same Windows message but reports a fraction of a line many times
/// a second, which is already smooth; easing it only adds lag.
const DIRECT_LINES: f32 = 1.0;

/// Offsets follow GPUI's convention: `0` is the top, growing more negative as
/// you scroll down.
#[derive(Clone)]
enum Surface {
    Div(ScrollHandle),
    List(ListState),
}

impl Surface {
    fn offset(&self) -> Pixels {
        match self {
            Self::Div(handle) => handle.offset().y,
            // A bottom-aligned list parks its anchor one item past the last one,
            // and this getter reports that anchor's top: the full content
            // height, a viewport short of the bottom it's actually drawn at.
            // Clamping folds that back onto the bottom.
            Self::List(state) => state
                .scroll_px_offset_for_scrollbar()
                .y
                .clamp(self.min_offset(), px(0.)),
        }
    }

    fn set_offset(&self, y: Pixels) {
        match self {
            Self::Div(handle) => handle.set_offset(point(handle.offset().x, y)),
            // Repins a bottom-aligned list when it lands at the bottom, which is
            // what makes new messages follow along.
            Self::List(state) => state.set_offset_from_scrollbar(point(px(0.), y)),
        }
    }

    fn min_offset(&self) -> Pixels {
        match self {
            Self::Div(handle) => -handle.max_offset().height,
            // Has to match the setter's own clamp to the last float, or a target
            // of `min_offset` stops short of the bottom and the list never
            // repins. It only does while the list element carries no padding:
            // the setter counts padding in the scrollable range and this getter
            // doesn't, so `py_2` there would leave us 16px short forever.
            Self::List(state) => -state.max_offset_for_scrollbar().height,
        }
    }
}

/// Eases a scroll container toward the position the wheel asked for.
///
/// Drive it with [`step`](Self::step) once per render and
/// [`absorb`](Self::absorb) from the container's `on_scroll_wheel`.
pub struct SmoothScroll {
    surface: Surface,
    /// Where the container is drawn this frame.
    current: Pixels,
    /// Where the wheel has asked it to end up.
    target: Pixels,
}

impl SmoothScroll {
    pub fn div() -> Self {
        Self::new(Surface::Div(ScrollHandle::new()))
    }

    pub fn list(state: ListState) -> Self {
        Self::new(Surface::List(state))
    }

    fn new(surface: Surface) -> Self {
        Self {
            surface,
            current: px(0.),
            target: px(0.),
        }
    }

    /// For `.track_scroll()`. Div surfaces only.
    pub fn handle(&self) -> &ScrollHandle {
        match &self.surface {
            Surface::Div(handle) => handle,
            Surface::List(_) => unreachable!("list surfaces are tracked by their ListState"),
        }
    }

    /// Advance the glide one frame, asking for another while distance remains.
    /// Call once per render, before building the element.
    pub fn step(&mut self, window: &Window) {
        if self.current == self.target {
            // Idle, so adopt wherever the container actually sits: something
            // else may have moved it, such as a new message pinning a
            // bottom-aligned list back to the bottom.
            self.current = self.surface.offset();
            self.target = self.current;
        } else {
            self.current = advance(self.current, self.target);
            self.surface.set_offset(self.current);
            request_step(window.current_view(), Instant::now() + MIN_STEP, window);
        }
    }

    /// Overwrite the jump the container's own handler just applied and fold this
    /// event's delta into the target instead. Call from `on_scroll_wheel` on the
    /// container (or, for a list, an ancestor), which bubbles after that handler.
    ///
    /// The delta comes off the event rather than from how far the container
    /// moved. Measuring the container looks equivalent and isn't: a list
    /// re-applies every delta since the last paint as one coalesced total, and
    /// coalescing *discards* the running total the moment a delta flips sign. A
    /// single stray opposite-sign tick — routine from a touchpad — would then
    /// measure as the whole gesture so far running backwards.
    ///
    /// Requests no redraw: the handler we're overwriting already notified the
    /// view.
    pub fn absorb(&mut self, event: &ScrollWheelEvent, window: &Window) {
        let (delta, smooth) = wheel_delta(&event.delta, window.line_height());
        if delta.is_zero() {
            return;
        }

        self.target = (self.target + delta).clamp(self.surface.min_offset(), px(0.));
        if !smooth {
            self.current = self.target;
        }
        // Nothing has been painted since the container moved, so its jump is
        // never seen.
        self.surface.set_offset(self.current);
    }
}

/// This event's vertical travel in pixels, and whether it wants easing.
fn wheel_delta(delta: &ScrollDelta, line_height: Pixels) -> (Pixels, bool) {
    match delta {
        // Pixel deltas only come from a touchpad, which is smooth already.
        ScrollDelta::Pixels(delta) => (delta.y, false),
        ScrollDelta::Lines(delta) => (line_height * delta.y, delta.y.abs() > DIRECT_LINES),
    }
}

/// One frame of ease-out toward `target`. Snapping the last step rather than
/// shrinking it guarantees the glide ends; crawling at float precision forever
/// would redraw the screen forever with it.
fn advance(current: Pixels, target: Pixels) -> Pixels {
    let remaining = target - current;
    if remaining.abs() < px(SETTLE) {
        target
    } else {
        current + remaining * EASE
    }
}

/// Ask for a redraw once `due` has passed, riding the vsync callback until then.
/// Skipped frames are free: without a notify there is nothing to draw.
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
    use gpui::{ScrollDelta, point, px};

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

        // What a Windows precision touchpad sends: a fraction of a line, often.
        let (delta, smooth) = wheel_delta(&ScrollDelta::Lines(point(0., -0.2)), line_height);
        assert_eq!(delta, px(-4.));
        assert!(!smooth, "easing a touchpad's own stream only adds lag");

        assert_eq!(
            wheel_delta(&ScrollDelta::Pixels(point(px(0.), px(-7.5))), line_height),
            (px(-7.5), false)
        );
    }
}
