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
            Self::List(state) => state.scroll_px_offset_for_scrollbar().y,
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
            Self::List(state) => -state.max_offset_for_scrollbar().height,
        }
    }

    /// How far this one wheel event asked to move, given where the container sat
    /// at the last paint (`base`) and how much we've already taken this frame.
    fn delta_from(&self, base: Pixels, taken: Pixels) -> Pixels {
        let moved = self.offset() - base;
        match self {
            // Adds each delta to wherever it finds the offset, and we always
            // hand it back at `base`, so it reports this event alone.
            Self::Div(_) => moved,
            // Re-applies every delta it has seen since the last paint, so drop
            // what we already counted this frame.
            Self::List(_) => moved - taken,
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
    /// `current` as of the last render — where a wheel event will find it.
    base: Pixels,
    /// Distance absorbed since the last render.
    taken: Pixels,
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
            base: px(0.),
            taken: px(0.),
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

        self.base = self.current;
        self.taken = px(0.);
    }

    /// Take back the jump the container's own handler just applied and fold it
    /// into the target instead. Call from `on_scroll_wheel` on the container (or,
    /// for a list, an ancestor), which bubbles after that handler.
    ///
    /// Requests no redraw: the handler we're undoing already notified the view.
    pub fn absorb(&mut self) {
        let delta = self.surface.delta_from(self.base, self.taken);
        if delta.is_zero() {
            return;
        }
        self.taken += delta;
        self.target = (self.target + delta).clamp(self.surface.min_offset(), px(0.));
        // Undo the jump. Nothing has been painted since, so it's never seen.
        self.surface.set_offset(self.current);
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
    use super::advance;
    use gpui::px;

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
        assert!(frames > 3, "landed in {frames} frame(s), no smoothing happened");
    }
}
