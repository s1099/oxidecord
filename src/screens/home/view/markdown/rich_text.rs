//! One wrapped run of formatted text: per-range styling, clickable and
//! tooltipped ranges, and custom emoji drawn inline.
//!
//! gpui's text elements can't do two things markdown needs. A highlight can't
//! change the font family, so inline code couldn't be monospace; the runs are
//! built here instead, at layout time, from the ambient text style — which is
//! only known then. And text can't hold an image, so each custom emoji leaves
//! a gap of blank space in the text, and the image is painted into wherever
//! that gap lands once the text has wrapped. A trailing [`Marker`] works the
//! same way, since a run can't change the font size either.
//!
//! A selectable text also reports where it was laid out, for the message
//! list's selection to hit-test, and paints its share of that selection.

use std::mem;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;

use crate::screens::home::selection::{TextKey, TextRegion, TextRegions};
use crate::ui::tooltip;

/// An em space plus a quarter-em space, about as wide as Discord's inline emoji.
pub(super) const EMOJI_PLACEHOLDER: &str = "\u{2003}\u{2005}";

/// A word char, so the invisible stand-in wraps as one word.
const MARKER_FILL: char = '.';

pub(super) struct Marker {
    pub label: SharedString,
    pub size: Pixels,
    pub color: Hsla,
    pub tooltip: Option<SharedString>,
}

struct ShapedMarker {
    start: usize,
    line: ShapedLine,
    /// Offset that lines its baseline up with the text's in the same line box.
    baseline_shift: Pixels,
    origin: Option<Point<Pixels>>,
}

/// Segments run end to end.
pub(super) struct Segment {
    pub len: usize,
    pub style: HighlightStyle,
    pub mono: bool,
}

type ClickListener = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub(super) struct Selectable {
    pub key: TextKey,
    pub regions: TextRegions,
    pub selected: Option<Range<usize>>,
    pub color: Hsla,
}

pub(super) struct RichText {
    id: ElementId,
    text: SharedString,
    segments: Vec<Segment>,
    mono_family: SharedString,
    clickable: Vec<Range<usize>>,
    on_click: Option<ClickListener>,
    tooltips: Vec<(Range<usize>, SharedString)>,
    emoji: Vec<(Range<usize>, SharedString)>,
    marker: Option<Marker>,
    shaped_marker: Option<ShapedMarker>,
    selectable: Option<Selectable>,
    image_cache: Entity<RetainAllImageCache>,
    inner: Option<Inner>,
    layout: TextLayout,
    placed: Vec<AnyElement>,
}

/// Plain when nothing is interactive: `InteractiveText` repaints its whole view
/// whenever the pointer crosses a character.
enum Inner {
    Plain(StyledText),
    Interactive(InteractiveText),
}

impl RichText {
    pub fn new(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        segments: Vec<Segment>,
        mono_family: SharedString,
        image_cache: &Entity<RetainAllImageCache>,
    ) -> Self {
        let text = text.into();
        debug_assert_eq!(
            segments.iter().map(|segment| segment.len).sum::<usize>(),
            text.len()
        );
        Self {
            id: id.into(),
            text,
            segments,
            mono_family,
            clickable: Vec::new(),
            on_click: None,
            tooltips: Vec::new(),
            emoji: Vec::new(),
            marker: None,
            shaped_marker: None,
            selectable: None,
            image_cache: image_cache.clone(),
            inner: None,
            layout: TextLayout::default(),
            placed: Vec::new(),
        }
    }

    pub fn on_click(
        mut self,
        ranges: Vec<Range<usize>>,
        listener: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        if !ranges.is_empty() {
            self.clickable = ranges;
            self.on_click = Some(Rc::new(listener));
        }
        self
    }

    pub fn tooltips(mut self, tooltips: Vec<(Range<usize>, SharedString)>) -> Self {
        self.tooltips = tooltips;
        self
    }

    pub fn emoji(mut self, emoji: Vec<(Range<usize>, SharedString)>) -> Self {
        self.emoji = emoji;
        self
    }

    pub fn marker(mut self, marker: Marker) -> Self {
        self.marker = Some(marker);
        self
    }

    pub fn selectable(mut self, selectable: Selectable) -> Self {
        self.selectable = Some(selectable);
        self
    }
}

impl Element for RichText {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let base = window.text_style();
        let mut runs: Vec<TextRun> = self
            .segments
            .iter()
            .map(|segment| {
                let mut style = base.clone();
                if segment.mono {
                    style.font_family = self.mono_family.clone();
                }
                style.highlight(segment.style).to_run(segment.len)
            })
            .collect();
        let mut text = self.text.clone();
        self.shaped_marker = None;
        if let Some(marker) = self.marker.take() {
            let (fill, shaped) = shape_marker(&marker, &base, self.text.len() + 1, window);
            let mut owned = String::with_capacity(self.text.len() + 1 + fill.len());
            owned.push_str(&self.text);
            owned.push(' ');
            owned.push_str(&fill);
            runs.push(base.to_run(1));
            // Not via `highlight`: it blends over the base, so transparent changes nothing.
            runs.push(TextRun {
                color: transparent_black(),
                ..base.to_run(fill.len())
            });
            if let Some(tooltip) = marker.tooltip {
                self.tooltips
                    .push((shaped.start..shaped.start + fill.len(), tooltip));
            }
            text = owned.into();
            self.shaped_marker = Some(shaped);
        }
        let styled = StyledText::new(text).with_runs(runs);
        self.layout = styled.layout().clone();

        let mut inner = if self.on_click.is_none() && self.tooltips.is_empty() {
            Inner::Plain(styled)
        } else {
            let mut text = InteractiveText::new(self.id.clone(), styled);
            if let Some(on_click) = self.on_click.take() {
                text = text.on_click(mem::take(&mut self.clickable), move |ix, window, cx| {
                    on_click(ix, window, cx)
                });
            }
            if !self.tooltips.is_empty() {
                let tooltips = mem::take(&mut self.tooltips);
                text = text.tooltip(move |index, window, cx| {
                    let (_, label) = tooltips.iter().find(|(range, _)| range.contains(&index))?;
                    Some(tooltip::text(label.clone())(window, cx))
                });
            }
            Inner::Interactive(text)
        };

        let (layout_id, ()) = match &mut inner {
            Inner::Plain(text) => text.request_layout(None, inspector_id, window, cx),
            Inner::Interactive(text) => text.request_layout(id, inspector_id, window, cx),
        };
        self.inner = Some(inner);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let mut hitbox = match self.inner.as_mut()? {
            Inner::Plain(text) => {
                text.prepaint(None, inspector_id, bounds, &mut (), window, cx);
                None
            }
            Inner::Interactive(text) => {
                Some(text.prepaint(id, inspector_id, bounds, &mut (), window, cx))
            }
        };
        if let Some(selectable) = &self.selectable {
            // Plain text has no hitbox; the selection needs one to tell a press on the
            // text from one on whatever covers it.
            let hitbox = hitbox
                .get_or_insert_with(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
                .clone();
            selectable.regions.borrow_mut().push(TextRegion {
                key: selectable.key,
                text: self.text.clone(),
                layout: self.layout.clone(),
                bounds,
                hitbox,
            });
        }

        // A gap split by a wrap has no one width, so it gets a line-height square
        // from where it starts.
        self.placed.clear();
        let line_height = self.layout.line_height();
        for (range, url) in &self.emoji {
            let Some(start) = self.layout.position_for_index(range.start) else {
                continue;
            };
            let width = match self.layout.position_for_index(range.end) {
                Some(end) if end.y == start.y && end.x > start.x => end.x - start.x,
                _ => line_height,
            };
            let side = width.min(line_height);
            let origin = point(
                start.x + (width - side) / 2.,
                start.y + (line_height - side) / 2.,
            );
            let mut image = img(url.clone())
                .image_cache(&self.image_cache)
                .size(side)
                .into_any_element();
            image.prepaint_as_root(origin, size(side, side).map(Into::into), window, cx);
            self.placed.push(image);
        }

        if let Some(marker) = &mut self.shaped_marker {
            marker.origin = self
                .layout
                .position_for_index(marker.start)
                .map(|start| point(start.x, start.y + marker.baseline_shift));
        }
        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(selectable) = &self.selectable {
            // Under the glyphs and run backgrounds so mention pills and code stay legible.
            if let Some(range) = selectable.selected.clone() {
                paint_selection(&self.layout, bounds, range, selectable.color, window);
            }
            // Before the inner text's paint, so a link's pointing hand wins.
            if let Some(hitbox) = hitbox.as_ref() {
                window.set_cursor_style(CursorStyle::IBeam, hitbox);
            }
        }
        match (self.inner.as_mut(), hitbox) {
            (Some(Inner::Plain(text)), _) => {
                text.paint(None, inspector_id, bounds, &mut (), &mut (), window, cx)
            }
            (Some(Inner::Interactive(text)), Some(hitbox)) => {
                text.paint(id, inspector_id, bounds, &mut (), hitbox, window, cx)
            }
            _ => {}
        }
        for image in &mut self.placed {
            image.paint(window, cx);
        }
        if let Some(marker) = &self.shaped_marker
            && let Some(origin) = marker.origin
        {
            let _ = marker.line.paint(
                origin,
                self.layout.line_height(),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
}

impl IntoElement for RichText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A row the range runs on past the end of gets a sliver more, standing for the
/// line break.
fn paint_selection(
    layout: &TextLayout,
    bounds: Bounds<Pixels>,
    range: Range<usize>,
    color: Hsla,
    window: &mut Window,
) {
    let line_height = layout.line_height();
    let line_break = line_height / 4.;
    let mut top = bounds.top();
    let mut line_start = 0;
    for line in layout.line_layouts() {
        let line_end = line_start + line.len();
        if line_start >= range.end {
            break;
        }
        if line_end >= range.start {
            let unwrapped = &line.unwrapped_layout;
            let wraps = line
                .wrap_boundaries()
                .iter()
                .map(|boundary| unwrapped.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index);
            let mut row_start = 0;
            for (row, row_end) in wraps.chain([line.len()]).enumerate() {
                let from = range.start.saturating_sub(line_start).max(row_start);
                let to = (range.end - line_start).min(row_end);
                let runs_on = range.end > line_start + row_end;
                if from < to || (from == to && runs_on && from == row_end) {
                    let origin_x = unwrapped.x_for_index(row_start);
                    let left = unwrapped.x_for_index(from) - origin_x;
                    let mut right = unwrapped.x_for_index(to) - origin_x;
                    if runs_on && row_end == line.len() {
                        right += line_break;
                    }
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(bounds.left() + left, top + line_height * row as f32),
                            point(bounds.left() + right, top + line_height * (row + 1) as f32),
                        ),
                        color,
                    ));
                }
                row_start = row_end;
            }
        }
        top += line.size(line_height).height;
        line_start = line_end + 1;
    }
}

fn shape_marker(
    marker: &Marker,
    base: &TextStyle,
    start: usize,
    window: &mut Window,
) -> (String, ShapedMarker) {
    let font_size = base.font_size.to_pixels(window.rem_size());
    let line_height = base.line_height_in_pixels(window.rem_size());
    let text_system = window.text_system();
    let label_run = TextRun {
        color: marker.color,
        ..base.to_run(marker.label.len())
    };
    let line = text_system.shape_line(marker.label.clone(), marker.size, &[label_run], None);
    let fill_char: SharedString = MARKER_FILL.to_string().into();
    let fill_run = base.to_run(fill_char.len());
    let fill_shape = text_system.shape_line(fill_char, font_size, &[fill_run], None);

    let count = if fill_shape.width > px(0.) {
        (line.width / fill_shape.width).ceil().max(1.) as usize
    } else {
        marker.label.chars().count()
    };
    // Both lines are centred in one line box, so baselines differ by each font's metrics.
    let baseline = |ascent: Pixels, descent: Pixels| (line_height - ascent - descent) / 2. + ascent;
    let baseline_shift =
        baseline(fill_shape.ascent, fill_shape.descent) - baseline(line.ascent, line.descent);
    (
        MARKER_FILL.to_string().repeat(count),
        ShapedMarker {
            start,
            line,
            baseline_shift,
            origin: None,
        },
    )
}
