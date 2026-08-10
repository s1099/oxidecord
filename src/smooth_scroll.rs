//! Browser like smooth scrolling for scroll containers.
//!
//! Windows reports one wheel notch as a single multi-line jump, and GPUI's
//! containers apply the whole thing in one frame that is the line-to-line
//! snapping. We can't stop those built-in handlers from running, so instead we
//! let them move the container, immediately put it back, and ease toward the
//! position they asked for over the following frames.
//!
//! Two containers to drive, with the same shape but different reset semantics:
//! `overflow_*_scroll` divs (via [`ScrollHandle`]) and the virtualised
//! [`ListState`]. See [`Surface::delta_from`] for where they differ.

use gpui::*;

/// Fraction of the distance still owed that gets handed over each frame. Lower
/// is floatier, higher approaches the original instant jump. At 60fps 0.3
/// settles in roughly 150ms, about what a browser does.
const EASE: f32 = 0.3;
/// Once this little is left, deliver the remainder rather than crawl toward it a
/// fraction at a time.
const SETTLE: f32 = 0.5;

/// The scrollable thing being animated. Offsets follow GPUI's convention: `0` is
/// the top and they grow more negative as you scroll down.
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
            // Keeps a bottom-aligned list pinned to the bottom when it lands
            // there, which is what makes new messages follow along.
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
            // The div's handler adds each delta to wherever it finds the offset,
            // and we always hand it back at `base`, so it reports one event.
            Self::Div(_) => moved,
            // The list's handler re-applies every delta it has seen since the
            // last paint, so subtract what we already counted this frame.
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
    /// Where the container is actually drawn this frame.
    current: Pixels,
    target: Pixels,
    /// `current` as of the last render - where a wheel event will find it.
    base: Pixels,
    /// Distance absorbed since the last render, for `Surface::delta_from`.
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

    /// The handle to hand to `.track_scroll()`. Div surfaces only.
    pub fn handle(&self) -> &ScrollHandle {
        match &self.surface {
            Surface::Div(handle) => handle,
            Surface::List(_) => unreachable!("list surfaces are tracked by their ListState"),
        }
    }

    /// Advance the animation one frame. Call once per render, before building
    /// the element, and it will ask for the next frame while it still has
    /// distance to cover.
    pub fn step(&mut self, window: &Window) {
        if self.current == self.target {
            // Idle, so adopt wherever the container actually sits: something
            // else may have moved it, such as a new message pinning a
            // bottom-aligned list back to the bottom.
            self.current = self.surface.offset();
            self.target = self.current;
        } else {
            let remaining = self.target - self.current;
            self.current += if remaining.abs() < px(SETTLE) {
                remaining
            } else {
                remaining * EASE
            };
            self.surface.set_offset(self.current);
            window.request_animation_frame();
        }

        self.base = self.current;
        self.taken = px(0.);
    }

    /// Take back a jump the container's own scroll handler just applied and fold
    /// it into the target instead. Call from `on_scroll_wheel` on the container
    /// (or, for a list, an ancestor), which bubbles after the built-in handler.
    ///
    /// No redraw is requested here: the handler we're undoing already notified
    /// the view, and `step` keeps asking for frames until the glide is done.
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

#[cfg(test)]
mod tests {
    use super::{EASE, SETTLE};
    use gpui::{IsZero as _, Pixels, px};

    fn ease(remaining: Pixels) -> Pixels {
        if remaining.abs() < px(SETTLE) {
            remaining
        } else {
            remaining * EASE
        }
    }

    #[test]
    fn easing_converges_on_the_exact_distance() {
        let mut remaining = px(-100.);
        let mut frames = 0;
        while !remaining.is_zero() {
            let step = ease(remaining);
            assert!(!step.is_zero(), "stalled with {remaining:?} left");
            remaining -= step;
            frames += 1;
            assert!(frames < 120, "never settled");
        }
        assert!(frames > 1, "delivered in one jump, no smoothing happened");
    }
}
