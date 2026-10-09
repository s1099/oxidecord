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

use std::mem;
use std::ops::Range;
use std::rc::Rc;

use gpui::*;

use crate::ui::tooltip;

/// The blank an emoji is drawn into: an em space and a quarter-em space, so
/// the gap comes out about as wide as Discord's inline emoji relative to its
/// text.
pub(super) const EMOJI_PLACEHOLDER: &str = "\u{2003}\u{2005}";

/// Stands in for a marker's label in the text: a word char, so the stand-in
/// wraps as one word, drawn invisible.
const MARKER_FILL: char = '.';

/// A small label set after the text, like "(edited)".
pub(super) struct Marker {
    pub label: SharedString,
    pub size: Pixels,
    pub color: Hsla,
    pub tooltip: Option<SharedString>,
}

/// A marker shaped in layout, ready to place once the text has wrapped.
struct ShapedMarker {
    /// Where its stand-in starts in the text.
    start: usize,
    line: ShapedLine,
    /// How far its baseline sits below where the text's would if it were
    /// painted in the same line box, so the two line up.
    baseline_shift: Pixels,
    /// Where it lands, from the last prepaint.
    origin: Option<Point<Pixels>>,
}

/// How much of the text a style covers. Segments run end to end.
pub(super) struct Segment {
    pub len: usize,
    pub style: HighlightStyle,
    pub mono: bool,
}

type ClickListener = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub(super) struct RichText {
    id: ElementId,
    text: SharedString,
    segments: Vec<Segment>,
    mono_family: SharedString,
    clickable: Vec<Range<usize>>,
    /// Called with the index into `clickable` of the range clicked.
    on_click: Option<ClickListener>,
    tooltips: Vec<(Range<usize>, SharedString)>,
    /// Each emoji's placeholder and image URL.
    emoji: Vec<(Range<usize>, SharedString)>,
    marker: Option<Marker>,
    shaped_marker: Option<ShapedMarker>,
    image_cache: Entity<RetainAllImageCache>,
    inner: Option<Inner>,
    layout: TextLayout,
    /// The emoji laid out in the last prepaint, ready to paint.
    placed: Vec<AnyElement>,
}

/// Plain text when nothing in it is interactive: `InteractiveText` repaints
/// its whole view every time the pointer crosses a character, which the
/// message list shouldn't pay for text with nothing to hover.
enum Inner {
    Plain(StyledText),
    Interactive(InteractiveText),
}

impl RichText {
    pub fn new(
        id: impl Into<ElementId>,
        text: String,
        segments: Vec<Segment>,
        mono_family: SharedString,
        image_cache: &Entity<RetainAllImageCache>,
    ) -> Self {
        debug_assert_eq!(
            segments.iter().map(|segment| segment.len).sum::<usize>(),
            text.len()
        );
        Self {
            id: id.into(),
            text: text.into(),
            segments,
            mono_family,
            clickable: Vec::new(),
            on_click: None,
            tooltips: Vec::new(),
            emoji: Vec::new(),
            marker: None,
            shaped_marker: None,
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
            // Set on the run, not via `highlight`: that blends the color over
            // the base, and transparent over anything leaves it unchanged.
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
        let hitbox = match self.inner.as_mut()? {
            Inner::Plain(text) => {
                text.prepaint(None, inspector_id, bounds, &mut (), window, cx);
                None
            }
            Inner::Interactive(text) => {
                Some(text.prepaint(id, inspector_id, bounds, &mut (), window, cx))
            }
        };

        // Each emoji is centred in its gap, as tall as it is wide but never
        // taller than the line. A gap split by a wrap has no one width, so
        // it gets a line-height square from where it starts.
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
            let _ = marker
                .line
                .paint(origin, self.layout.line_height(), window, cx);
        }
    }
}

impl IntoElement for RichText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Shapes a marker's label at its own size, and the invisible stand-in that
/// holds a gap as wide for it in text set in `base`.
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
    // Both lines are painted centred in the same line box, so the label's
    // baseline drops by the difference in where each font puts its own.
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
