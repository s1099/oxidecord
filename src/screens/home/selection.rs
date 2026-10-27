//! The mouse selection over the message list's text.
//!
//! One selection can span blocks, embeds and messages. Every selectable text has
//! a [`TextKey`] that sorts in reading order, so a selection is two
//! [`TextPoint`]s and each text finds its share by comparing keys.
//!
//! Hit-testing uses the [`TextRegion`]s the list lays out. Copying uses the
//! [`TextCopy`]s it draws, kept until the conversation closes so a selection
//! still copies whole after its middle scrolls out of view.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;

/// Reading order: message, then part, then index within the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct TextKey {
    pub message: u64,
    pub part: u32,
    pub index: u32,
}

/// In draw order.
#[derive(Clone, Copy)]
pub(super) enum TextPart {
    Forward,
    Content,
    Embed { embed: usize, piece: EmbedPiece },
}

#[derive(Clone, Copy)]
pub(super) enum EmbedPiece {
    Title,
    Description,
    FieldName(usize),
    FieldValue(usize),
}

/// Title, description, and two pieces for each of Discord's 25 fields.
const EMBED_SLOTS: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TextOwner {
    message: u64,
    part: u32,
}

impl TextOwner {
    pub fn new(message: u64, part: TextPart) -> Self {
        let part = match part {
            TextPart::Forward => 0,
            TextPart::Content => 1,
            TextPart::Embed { embed, piece } => {
                let slot = match piece {
                    EmbedPiece::Title => 0,
                    EmbedPiece::Description => 1,
                    EmbedPiece::FieldName(field) => 2 + 2 * field as u32,
                    EmbedPiece::FieldValue(field) => 3 + 2 * field as u32,
                };
                2 + embed as u32 * EMBED_SLOTS + slot
            }
        };
        Self { message, part }
    }

    pub fn key(self, index: usize) -> TextKey {
        TextKey {
            message: self.message,
            part: self.part,
            index: index as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TextPoint {
    pub key: TextKey,
    pub offset: usize,
}

/// Single click a point, double click the word, triple click the whole text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SelectUnit {
    Char,
    Word,
    Text,
}

impl SelectUnit {
    pub fn for_clicks(click_count: usize) -> Self {
        match click_count {
            0 | 1 => Self::Char,
            2 => Self::Word,
            _ => Self::Text,
        }
    }
}

pub(super) struct Hit {
    pub point: TextPoint,
    pub text: SharedString,
}

impl Hit {
    fn span(&self, unit: SelectUnit) -> (TextPoint, TextPoint) {
        let at = |offset| TextPoint {
            key: self.point.key,
            offset,
        };
        match unit {
            SelectUnit::Char => (self.point, self.point),
            SelectUnit::Word => {
                let word = word_at(&self.text, self.point.offset);
                (at(word.start), at(word.end))
            }
            SelectUnit::Text => (at(0), at(self.text.len())),
        }
    }
}

pub(super) struct Selection {
    pub anchor: TextPoint,
    pub head: TextPoint,
    /// What the press took; dragging grows from it and never shrinks below it.
    origin: (TextPoint, TextPoint),
    unit: SelectUnit,
    pub drag: Option<Drag>,
}

pub(super) struct Drag {
    /// Lets scrolling under a still pointer carry the selection on.
    pub pointer: Point<Pixels>,
    pub viewport: Bounds<Pixels>,
    pub autoscrolling: bool,
}

impl Selection {
    pub fn press(hit: &Hit, unit: SelectUnit, drag: Drag) -> Self {
        let origin = hit.span(unit);
        Self {
            anchor: origin.0,
            head: origin.1,
            origin,
            unit,
            drag: Some(drag),
        }
    }

    pub fn shift_press(&mut self, hit: &Hit, drag: Drag) {
        self.origin = (self.anchor, self.anchor);
        self.unit = SelectUnit::Char;
        self.drag = Some(drag);
        self.extend(hit);
    }

    /// Returns whether anything changed.
    pub fn extend(&mut self, hit: &Hit) -> bool {
        let (start, end) = hit.span(self.unit);
        let (anchor, head) = if start < self.origin.0 {
            (self.origin.1, start)
        } else {
            (self.origin.0, end.max(self.origin.1))
        };
        let changed = (anchor, head) != (self.anchor, self.head);
        self.anchor = anchor;
        self.head = head;
        changed
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    fn ordered(&self) -> (TextPoint, TextPoint) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub fn range_in(&self, key: TextKey, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if key < start.key || key > end.key {
            return None;
        }
        let from = if key == start.key { start.offset } else { 0 };
        let to = if key == end.key { end.offset } else { len };
        let range = from.min(len)..to.min(len);
        (range.start < range.end).then_some(range)
    }
}

/// Custom emoji are drawn into blanks in the text but copy as `:name:`, as in Discord.
pub(super) struct TextCopy {
    text: SharedString,
    emoji: Vec<(Range<usize>, SharedString)>,
}

impl TextCopy {
    pub fn new(text: SharedString, emoji: Vec<(Range<usize>, SharedString)>) -> Self {
        Self { text, emoji }
    }

    fn slice(&self, range: Range<usize>) -> String {
        let text = self.text.as_str();
        let end = ceil_char_boundary(text, range.end.min(text.len()));
        let mut at = floor_char_boundary(text, range.start.min(end));
        let mut copied = String::with_capacity(end - at);
        for (blank, name) in &self.emoji {
            if blank.end <= at || blank.start >= end {
                continue;
            }
            copied.push_str(&text[at..blank.start.max(at)]);
            copied.push_str(name);
            at = blank.end.min(end);
        }
        copied.push_str(&text[at..end]);
        copied
    }
}

#[derive(Default)]
pub(super) struct TextCopies(BTreeMap<TextKey, TextCopy>);

impl TextCopies {
    pub fn insert(&mut self, key: TextKey, copy: TextCopy) {
        self.0.insert(key, copy);
    }

    /// Called before redrawing a part so texts lost to an edit don't linger.
    pub fn clear_part(&mut self, owner: TextOwner) {
        let start = owner.key(0);
        let end = TextKey {
            index: u32::MAX,
            ..start
        };
        while let Some(key) = self.0.range(start..=end).next().map(|(key, _)| *key) {
            self.0.remove(&key);
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// One line per text. `keep` drops messages no longer in the conversation.
    pub fn copy(&self, selection: &Selection, keep: impl Fn(u64) -> bool) -> String {
        let (start, end) = selection.ordered();
        let mut copied = String::new();
        for (key, text) in self.0.range(start.key..=end.key) {
            if !keep(key.message) {
                continue;
            }
            let Some(range) = selection.range_in(*key, text.text.len()) else {
                continue;
            };
            if !copied.is_empty() {
                copied.push('\n');
            }
            copied.push_str(&text.slice(range));
        }
        copied
    }
}

#[derive(Clone)]
pub(super) struct TextRegion {
    pub key: TextKey,
    /// Without any trailing marker.
    pub text: SharedString,
    pub layout: TextLayout,
    pub bounds: Bounds<Pixels>,
    pub hitbox: Hitbox,
}

/// Rebuilt each frame drawn; a frame replayed from the cache keeps the last set.
pub(super) type TextRegions = Rc<RefCell<Vec<TextRegion>>>;

impl TextRegion {
    /// Above the text is its start, below its end. `closest` rounds to the
    /// nearer side of the character as a caret does; otherwise it's the
    /// character's start, for finding the word.
    fn offset_at(&self, position: Point<Pixels>, closest: bool) -> usize {
        let bounds = self.bounds;
        let len = self.text.len();
        if position.y < bounds.top() {
            return 0;
        }
        if position.y >= bounds.bottom() {
            return len;
        }
        let line_height = self.layout.line_height();
        let mut top = bounds.top();
        let mut start = 0;
        for line in self.layout.line_layouts() {
            let height = line.size(line_height).height;
            if position.y < top + height {
                let local = point(position.x - bounds.left(), position.y - top);
                let found = if closest {
                    line.closest_index_for_position(local, line_height)
                } else {
                    line.index_for_position(local, line_height)
                };
                let (Ok(index) | Err(index)) = found;
                return (start + index).min(len);
            }
            top += height;
            start += line.len() + 1;
        }
        len
    }

    fn hit(&self, position: Point<Pixels>, closest: bool) -> Hit {
        Hit {
            point: TextPoint {
                key: self.key,
                offset: self.offset_at(position, closest),
            },
            text: self.text.clone(),
        }
    }
}

/// Only a text the pointer is over and nothing covers.
pub(super) fn hit_under(
    regions: &[TextRegion],
    position: Point<Pixels>,
    unit: SelectUnit,
    window: &Window,
) -> Option<Hit> {
    regions
        .iter()
        .find(|region| region.hitbox.is_hovered(window))
        .map(|region| region.hit(position, unit == SelectUnit::Char))
}

/// Off any text, the nearest one.
pub(super) fn hit_nearest(regions: &[TextRegion], position: Point<Pixels>) -> Option<Hit> {
    let distance = |region: &TextRegion| {
        let bounds = region.bounds;
        let dy = (bounds.top() - position.y).max(position.y - bounds.bottom() + px(1.));
        let dx = (bounds.left() - position.x).max(position.x - bounds.right());
        // Vertical distance dominates, so a pointer beside a line picks that line.
        (dy.max(px(0.)).as_f32(), dx.max(px(0.)).as_f32())
    };
    regions
        .iter()
        .min_by(|a, b| distance(a).partial_cmp(&distance(b)).unwrap())
        .map(|region| region.hit(position, true))
}

#[derive(PartialEq)]
enum CharClass {
    Space,
    Word,
    Punctuation,
}

impl CharClass {
    fn of(c: char) -> Self {
        if c.is_whitespace() {
            Self::Space
        } else if c.is_alphanumeric() || c == '_' {
            Self::Word
        } else {
            Self::Punctuation
        }
    }
}

/// A word, a run of space, or a run of punctuation.
pub(super) fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = floor_char_boundary(text, offset.min(text.len()));
    // At the very end, the character before it.
    let Some((at, c)) = text[offset..]
        .chars()
        .next()
        .map(|c| (offset, c))
        .or_else(|| text[..offset].char_indices().next_back())
    else {
        return offset..offset;
    };
    let class = CharClass::of(c);
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| CharClass::of(*c) == class)
        .last()
        .map_or(at, |(ix, _)| ix);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| CharClass::of(*c) != class)
        .map_or(text.len(), |(ix, _)| at + ix);
    start..end
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    // Not a glob: it would bring gpui's `test` attribute in over the standard one.
    use super::{
        Bounds, Drag, EmbedPiece, Hit, Point, SelectUnit, Selection, TextCopies, TextCopy, TextKey,
        TextOwner, TextPart, TextPoint, word_at,
    };

    fn key(message: u64, index: usize) -> TextKey {
        TextOwner::new(message, TextPart::Content).key(index)
    }

    fn point(message: u64, index: usize, offset: usize) -> TextPoint {
        TextPoint {
            key: key(message, index),
            offset,
        }
    }

    fn hit(message: u64, index: usize, offset: usize, text: &'static str) -> Hit {
        Hit {
            point: point(message, index, offset),
            text: text.into(),
        }
    }

    fn drag() -> Drag {
        Drag {
            pointer: Point::default(),
            viewport: Bounds::default(),
            autoscrolling: false,
        }
    }

    #[test]
    fn keys_sort_in_reading_order() {
        let embed = |embed, piece| TextOwner::new(1, TextPart::Embed { embed, piece }).key(0);
        let order = [
            TextOwner::new(1, TextPart::Forward).key(3),
            key(1, 0),
            key(1, 9),
            embed(0, EmbedPiece::Title),
            embed(0, EmbedPiece::Description),
            embed(0, EmbedPiece::FieldName(0)),
            embed(0, EmbedPiece::FieldValue(0)),
            embed(0, EmbedPiece::FieldName(24)),
            embed(0, EmbedPiece::FieldValue(24)),
            embed(1, EmbedPiece::Title),
            TextOwner::new(2, TextPart::Forward).key(0),
        ];
        assert!(order.is_sorted());
    }

    #[test]
    fn selection_covers_the_texts_between_its_ends() {
        let mut selection =
            Selection::press(&hit(1, 0, 4, "hello world"), SelectUnit::Char, drag());
        assert!(selection.is_empty());
        assert_eq!(selection.range_in(key(1, 0), 11), None);

        selection.extend(&hit(3, 1, 2, "abc"));
        assert_eq!(selection.range_in(key(1, 0), 11), Some(4..11));
        assert_eq!(selection.range_in(key(2, 0), 5), Some(0..5));
        assert_eq!(selection.range_in(key(3, 0), 6), Some(0..6));
        assert_eq!(selection.range_in(key(3, 1), 3), Some(0..2));
        assert_eq!(selection.range_in(key(3, 2), 3), None);
        assert_eq!(selection.range_in(key(0, 0), 3), None);

        selection.extend(&hit(1, 0, 1, "hello world"));
        assert_eq!(selection.range_in(key(1, 0), 11), Some(1..4));
        assert_eq!(selection.range_in(key(2, 0), 5), None);
    }

    #[test]
    fn a_word_selection_grows_a_word_at_a_time() {
        let text = "one two three";
        let mut selection = Selection::press(&hit(1, 0, 5, text), SelectUnit::Word, drag());
        assert_eq!(selection.range_in(key(1, 0), text.len()), Some(4..7));

        selection.extend(&hit(1, 0, 9, text));
        assert_eq!(selection.range_in(key(1, 0), text.len()), Some(4..13));

        selection.extend(&hit(1, 0, 1, text));
        assert_eq!(selection.range_in(key(1, 0), text.len()), Some(0..7));
    }

    #[test]
    fn shift_press_extends_from_the_anchor() {
        let mut selection = Selection::press(&hit(1, 0, 2, "abcdef"), SelectUnit::Char, drag());
        selection.extend(&hit(1, 0, 4, "abcdef"));
        selection.shift_press(&hit(2, 0, 1, "xyz"), drag());
        assert_eq!(selection.range_in(key(1, 0), 6), Some(2..6));
        assert_eq!(selection.range_in(key(2, 0), 3), Some(0..1));
    }

    #[test]
    fn words_are_runs_of_like_characters() {
        let text = "hi, there  friend!!";
        assert_eq!(word_at(text, 0), 0..2);
        assert_eq!(word_at(text, 1), 0..2);
        assert_eq!(word_at(text, 2), 2..3);
        assert_eq!(word_at(text, 5), 4..9);
        assert_eq!(word_at(text, 9), 9..11);
        assert_eq!(word_at(text, text.len()), 17..19);
        assert_eq!(word_at("", 0), 0..0);
        assert_eq!(word_at("héllo wörld", 8), 7..13);
    }

    #[test]
    fn copies_join_texts_by_line_and_name_emoji() {
        let mut copies = TextCopies::default();
        let blank = "\u{2003}\u{2005}";
        let first = format!("hi {blank} there");
        let start = 3;
        let emoji = vec![(start..start + blank.len(), ":wave:".into())];
        copies.insert(key(1, 0), TextCopy::new(first.into(), emoji));
        copies.insert(key(1, 1), TextCopy::new("second".into(), Vec::new()));
        copies.insert(key(2, 0), TextCopy::new("deleted".into(), Vec::new()));
        copies.insert(key(3, 0), TextCopy::new("last one".into(), Vec::new()));

        let mut selection = Selection::press(&hit(1, 0, 0, ""), SelectUnit::Char, drag());
        selection.extend(&hit(3, 0, 4, ""));
        assert_eq!(
            copies.copy(&selection, |message| message != 2),
            "hi :wave: there\nsecond\nlast"
        );

        let mut selection = Selection::press(&hit(1, 0, start + 1, ""), SelectUnit::Char, drag());
        selection.extend(&hit(1, 0, start + blank.len() + 3, ""));
        assert_eq!(copies.copy(&selection, |_| true), ":wave: th");
    }

    #[test]
    fn clearing_a_part_leaves_the_rest() {
        let mut copies = TextCopies::default();
        let owner = TextOwner::new(1, TextPart::Content);
        copies.insert(owner.key(0), TextCopy::new("a".into(), Vec::new()));
        copies.insert(owner.key(1), TextCopy::new("b".into(), Vec::new()));
        copies.insert(key(2, 0), TextCopy::new("c".into(), Vec::new()));
        copies.clear_part(owner);

        let mut selection = Selection::press(&hit(1, 0, 0, ""), SelectUnit::Char, drag());
        selection.extend(&hit(2, 0, 1, ""));
        assert_eq!(copies.copy(&selection, |_| true), "c");
    }
}
