//! One mouse selection over a message's text, wherever on screen it is.
//!
//! The selection lives in an app global rather than element state, for two
//! reasons: a press anywhere has to be able to clear it, and Ctrl+C has to
//! reach it whatever holds focus — key listeners registered by an element
//! only run along the focused element's dispatch path, which never contains
//! the message list. The global holds the owner's text alongside the
//! selection, so copying never needs the element that drew it.

use std::ops::Range;

use gpui::*;

/// The selection over one rich-text element's text, if any, and its copy
/// hook.
#[derive(Default)]
pub(super) struct MessageSelection {
    active: Option<Active>,
    /// The observer that copies on Ctrl+C (Cmd+C on macOS). Built with the
    /// first selection and kept after it clears: idle, it costs one
    /// early-returning key check per keystroke, and re-building it per
    /// selection would hold a subscription inside a global for nothing.
    copy_on_c: Option<Subscription>,
}

/// An active selection: whose text, and its two ends.
struct Active {
    /// The owning element's id, unique across the message list.
    owner: ElementId,
    /// The owner's text, shared with it, so copying needs no element.
    text: SharedString,
    /// The ends, in byte indices: where the press landed, and where the
    /// drag has taken it. Equal ends are an empty selection.
    anchor: usize,
    head: usize,
}

impl Global for MessageSelection {}

/// The selected byte range of the element with `owner`, low end first, or
/// `None` when what is selected belongs to another text.
pub(super) fn selected_range(owner: &ElementId, cx: &App) -> Option<Range<usize>> {
    let active = cx.try_global::<MessageSelection>()?.active.as_ref()?;
    if active.owner != *owner {
        return None;
    }
    let start = active.anchor.min(active.head);
    let end = active.anchor.max(active.head);
    (start < end).then_some(start..end)
}

/// Draws the selection's highlight behind the text, one bar per wrapped
/// line: the anchor's x to the head's x on the end lines, the element's
/// full width across the lines in between, whose text reaches it anyway.
pub(super) fn paint_highlight(
    layout: &TextLayout,
    range: Range<usize>,
    color: Hsla,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let line_height = layout.line_height();
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    let lines = ((end.y - start.y) / line_height).round() as usize + 1;
    for line in 0..lines {
        let left = if line == 0 { start.x } else { bounds.left() };
        let right = if line + 1 == lines {
            end.x
        } else {
            bounds.right()
        };
        if right <= left {
            continue;
        }
        let y = start.y + line_height * line as f32;
        window.paint_quad(fill(
            Bounds {
                origin: point(left, y),
                size: size(right - left, line_height),
            },
            color,
        ));
    }
}

/// Wires one selectable rich-text element into the selection for this
/// frame. A press inside its hitbox starts a selection there; dragging with
/// the button held moves the head, across wrapped lines; releasing keeps
/// what was dragged, unless nothing was, which clears; a press anywhere
/// else dismisses whatever selection there was.
pub(super) fn attach(
    owner: ElementId,
    text: SharedString,
    layout: &TextLayout,
    hitbox: &Hitbox,
    window: &mut Window,
    cx: &mut App,
) {
    let is_owner = cx
        .try_global::<MessageSelection>()
        .and_then(|selection| selection.active.as_ref())
        .is_some_and(|active| active.owner == owner);
    let max_index = layout.len();

    window.on_mouse_event({
        let owner = owner.clone();
        let text = text.clone();
        let layout = layout.clone();
        let hitbox = hitbox.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if event.button != MouseButton::Left {
                return;
            }
            if phase == DispatchPhase::Capture {
                // Capture runs every handler before any bubble handler, so a
                // press anywhere off the text dismisses the old selection
                // without racing the press that replaces it. The owner may
                // not even be mounted anymore, after a channel switch.
                let has_selection = cx
                    .try_global::<MessageSelection>()
                    .is_some_and(|selection| selection.active.is_some());
                if has_selection && !hitbox.is_hovered(window) {
                    cx.default_global::<MessageSelection>().active = None;
                    window.refresh();
                }
                return;
            }
            if hitbox.is_hovered(window) {
                let index = hit_index(&layout, event.position, max_index);
                let needs_observer = {
                    let selection = cx.default_global::<MessageSelection>();
                    selection.active = Some(Active {
                        owner: owner.clone(),
                        text: text.clone(),
                        anchor: index,
                        head: index,
                    });
                    selection.copy_on_c.is_none()
                };
                if needs_observer {
                    let copy_on_c = cx.observe_keystrokes(copy_on_keystroke);
                    cx.default_global::<MessageSelection>().copy_on_c = Some(copy_on_c);
                }
                window.refresh();
            }
        }
    });

    if !is_owner {
        return;
    }
    window.on_mouse_event({
        let owner = owner.clone();
        let layout = layout.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !event.dragging() {
                return;
            }
            let head = hit_index(&layout, event.position, max_index);
            let moved = {
                let selection = cx.default_global::<MessageSelection>();
                match selection.active.as_mut() {
                    Some(active) if active.owner == owner && active.head != head => {
                        active.head = head;
                        true
                    }
                    _ => false,
                }
            };
            if moved {
                window.refresh();
            }
        }
    });
    window.on_mouse_event({
        let owner = owner.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let selection = cx.default_global::<MessageSelection>();
            let empty = selection
                .active
                .as_ref()
                .is_some_and(|active| active.owner == owner && active.anchor == active.head);
            if empty {
                // A press that never dragged selects nothing.
                selection.active = None;
                window.refresh();
            }
        }
    });
}

/// The text index a position points at. `index_for_position` reports the
/// nearest line boundary in its `Err`, which is the right answer past a
/// line's ends, above and below the text; clamped in case the text has
/// shrunk under an older selection.
fn hit_index(layout: &TextLayout, position: Point<Pixels>, max: usize) -> usize {
    layout
        .index_for_position(position)
        .unwrap_or_else(|boundary| boundary)
        .min(max)
}

/// Copies the active selection's visible text: the flattened markdown
/// output, where a mention copies as its shown name and an emoji's
/// placeholder as plain space. Typing plain `c` into the composer lands
/// here too; the modifier check sends it on its way.
fn copy_on_keystroke(event: &KeystrokeEvent, _: &mut Window, cx: &mut App) {
    let modifiers = &event.keystroke.modifiers;
    if event.keystroke.key != "c" || !(modifiers.control || modifiers.platform) {
        return;
    }
    let Some(text) = cx.try_global::<MessageSelection>().and_then(|selection| {
        let active = selection.active.as_ref()?;
        let text = &active.text;
        let start = snap_back(text, active.anchor.min(active.head));
        let end = snap_forward(text, active.anchor.max(active.head)).min(text.len());
        (start < end).then(|| text[start..end].to_string())
    }) else {
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(text));
}

/// Byte indices from hit-testing land on glyph starts, which are char
/// boundaries; these guard the copy against an odd cluster edge anyway.
fn snap_back(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn snap_forward(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}
