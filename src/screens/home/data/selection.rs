//! Pressing, dragging and copying the mouse selection over the message list.
//! What a selection is lives in [`crate::screens::home::selection`]; the
//! list's view feeds the mouse in here.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::*;

use crate::screens::home::selection::{self, Drag, SelectUnit, Selection};
use crate::screens::home::{CopySelection, HomeScreen};

/// How often a drag held past the list's top or bottom scrolls it on.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);
/// How far each tick scrolls for every pixel the pointer is past the edge,
/// and the most it goes, so pulling further away scrolls faster.
const AUTOSCROLL_RATE: f32 = 0.25;
const AUTOSCROLL_MAX: f32 = 24.;

impl HomeScreen {
    pub(in crate::screens::home) fn has_text_selection(&self) -> bool {
        self.text_selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// A left press anywhere in the window. On the list's text it starts a
    /// selection there, or with shift held moves the current one's head
    /// there; anywhere else it drops the selection.
    pub(in crate::screens::home) fn press_text(
        &mut self,
        event: &MouseDownEvent,
        viewport: Bounds<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let was_shown = self.has_text_selection();
        let unit = SelectUnit::for_clicks(event.click_count);
        let hit = selection::hit_under(&self.text_regions.borrow(), event.position, unit, window);
        let drag = Drag {
            pointer: event.position,
            viewport,
            autoscrolling: false,
        };
        if let Some(hit) = hit {
            match &mut self.text_selection {
                Some(selection) if event.modifiers.shift => selection.shift_press(&hit, drag),
                _ => self.text_selection = Some(Selection::press(&hit, unit, drag)),
            }
        } else {
            self.text_selection = None;
        }
        if was_shown || self.has_text_selection() {
            self.redraw_messages(cx);
        }
    }

    pub(in crate::screens::home) fn is_dragging_text(&self) -> bool {
        self.text_selection
            .as_ref()
            .is_some_and(|selection| selection.drag.is_some())
    }

    /// The pointer moving with the button held: the selection follows it,
    /// and past the list's top or bottom the list scrolls to meet it.
    pub(in crate::screens::home) fn drag_text(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag_mut() else {
            return;
        };
        drag.pointer = position;
        let past_edge = overshoot(drag) != px(0.);
        if self.follow_pointer() {
            self.redraw_messages(cx);
        }
        if past_edge {
            self.start_autoscroll(cx);
        }
    }

    /// The button coming up, wherever it is. A press that never dragged
    /// selects nothing.
    pub(in crate::screens::home) fn release_text(&mut self) {
        let Some(selection) = &mut self.text_selection else {
            return;
        };
        selection.drag = None;
        if selection.is_empty() {
            self.text_selection = None;
        }
    }

    pub(in crate::screens::home) fn on_copy_selection(
        &mut self,
        _: &CopySelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = self
            .text_selection
            .as_ref()
            .filter(|selection| !selection.is_empty())
        else {
            cx.propagate();
            return;
        };
        let loaded: HashSet<u64> = self
            .messages
            .iter()
            .map(|message| message.id.get())
            .collect();
        let text = self
            .text_copies
            .borrow()
            .copy(selection, |message| loaded.contains(&message));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn drag_mut(&mut self) -> Option<&mut Drag> {
        self.text_selection.as_mut()?.drag.as_mut()
    }

    /// Carries the dragged end to the pointer, held inside the list's visible
    /// part, so a pointer past an edge reaches the first or last line
    /// showing. Returns whether the selection changed.
    fn follow_pointer(&mut self) -> bool {
        let Some(selection) = &mut self.text_selection else {
            return false;
        };
        let Some(drag) = &selection.drag else {
            return false;
        };
        let viewport = drag.viewport;
        let position = point(
            drag.pointer.x,
            drag.pointer
                .y
                .clamp(viewport.top(), viewport.bottom() - px(1.)),
        );
        let Some(hit) = selection::hit_nearest(&self.text_regions.borrow(), position) else {
            return false;
        };
        selection.extend(&hit)
    }

    /// Scrolls the list a step at a time for as long as the drag stays past
    /// its edge. Mouse moves alone can't drive it: a pointer held still past
    /// the edge sends none.
    fn start_autoscroll(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag_mut() else {
            return;
        };
        if drag.autoscrolling {
            return;
        }
        drag.autoscrolling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_TICK).await;
                let scrolling = this
                    .update(cx, |this, cx| this.autoscroll_step(cx))
                    .unwrap_or(false);
                if !scrolling {
                    break;
                }
            }
        })
        .detach();
    }

    /// One tick of autoscroll. Returns whether to keep going.
    fn autoscroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.drag_mut() else {
            return false;
        };
        let past = overshoot(drag);
        if past == px(0.) {
            drag.autoscrolling = false;
            return false;
        }
        let step = (past * AUTOSCROLL_RATE).clamp(px(-AUTOSCROLL_MAX), px(AUTOSCROLL_MAX));
        // Offsets grow more negative further down, so a pointer below the
        // list, past it by a positive amount, takes the offset down.
        let before = self.messages_scroll.offset();
        self.messages_scroll.jump_to(before - step);
        // At the end it can't go past, keep waiting on the pointer without
        // redrawing a list that hasn't moved.
        if self.messages_scroll.offset() != before {
            // Hit-tests the frame before this scroll, a tick behind; the next
            // tick catches up.
            self.follow_pointer();
            self.redraw_messages(cx);
        }
        true
    }

    /// Redraws only the message list, which is all a selection changes.
    fn redraw_messages(&self, cx: &mut Context<Self>) {
        self.message_list_view.update(cx, |_, cx| cx.notify());
    }
}

/// How far the pointer is above the list (negative) or below it (positive).
fn overshoot(drag: &Drag) -> Pixels {
    let (y, viewport) = (drag.pointer.y, drag.viewport);
    if y < viewport.top() {
        y - viewport.top()
    } else if y > viewport.bottom() {
        y - viewport.bottom()
    } else {
        px(0.)
    }
}
