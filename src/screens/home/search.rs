//! Message search's query language, after Discord's: `key:value` filters
//! (`from:`, `has:`, `before:` and so on) mixed with the words to look for.
//!
//! Parsing is pure; names (`from:alice`, `in:general`) are left for the
//! caller to resolve against what the screen knows.

use std::collections::HashMap;
use std::ops::Range;

use chrono::{Days, Local, NaiveDate, TimeZone as _};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, MessageMarker, UserMarker},
};

use gpui_kit::{ListState, Task};

use crate::discord::{
    AuthorType, SEARCH_MAX_OFFSET, SEARCH_PAGE_SIZE, SearchFilters, SearchHas, SearchHit,
    SearchScope,
};

use super::autocomplete::Suggestion;

/// Milliseconds from the Unix epoch to Discord's (2015-01-01).
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FilterKind {
    From,
    Mentions,
    Has,
    Before,
    During,
    After,
    In,
    Pinned,
    AuthorType,
}

impl FilterKind {
    /// In the order Discord's search options list them.
    pub const ALL: [Self; 9] = [
        Self::From,
        Self::Mentions,
        Self::Has,
        Self::Before,
        Self::During,
        Self::After,
        Self::In,
        Self::Pinned,
        Self::AuthorType,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::Mentions => "mentions",
            Self::Has => "has",
            Self::Before => "before",
            Self::During => "during",
            Self::After => "after",
            Self::In => "in",
            Self::Pinned => "pinned",
            Self::AuthorType => "authorType",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::From | Self::Mentions => "user",
            Self::Has => "link, embed or file",
            Self::Before | Self::During | Self::After => "specific date",
            Self::In => "channel",
            Self::Pinned => "true or false",
            Self::AuthorType => "user, bot or webhook",
        }
    }

    /// Only a server has channels to pick between.
    pub fn in_dms(self) -> bool {
        self != Self::In
    }
}

/// One whitespace-separated piece of the query.
#[derive(PartialEq, Eq, Debug)]
pub(super) struct Term<'a> {
    /// `None` for a word to look for.
    pub kind: Option<FilterKind>,
    /// Quotes stripped.
    pub value: &'a str,
    /// Byte range of the whole term, key included.
    pub range: Range<usize>,
    /// Where the value starts.
    pub value_start: usize,
}

pub(super) fn terms(text: &str) -> Vec<Term<'_>> {
    let mut terms = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let Some(c) = rest.chars().next() else { break };
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let kind = FilterKind::ALL.into_iter().find(|kind| {
            let key = kind.key();
            rest.len() > key.len()
                && rest.as_bytes()[key.len()] == b':'
                && rest[..key.len()].eq_ignore_ascii_case(key)
        });
        let value_start = kind.map_or(i, |kind| i + kind.key().len() + 1);
        let (value, end) = match text[value_start..].strip_prefix('"') {
            // A name with spaces, which only a filter takes quoted.
            Some(quoted) if kind.is_some() => match quoted.find('"') {
                Some(close) => (&quoted[..close], value_start + close + 2),
                None => (quoted, text.len()),
            },
            _ => {
                let len = text[value_start..]
                    .find(char::is_whitespace)
                    .unwrap_or(text.len() - value_start);
                (&text[value_start..value_start + len], value_start + len)
            }
        };
        terms.push(Term {
            kind,
            value,
            range: i..end,
            value_start,
        });
        i = end;
    }
    terms
}

/// What's being typed at the cursor, for the suggestions under the box.
#[derive(PartialEq, Eq, Debug)]
pub(super) enum Editing<'a> {
    /// A bare word, possibly the start of a key; empty after a space.
    Key {
        prefix: &'a str,
        range: Range<usize>,
    },
    Value {
        kind: FilterKind,
        /// Up to the cursor.
        value: &'a str,
        range: Range<usize>,
    },
}

pub(super) fn editing_at(text: &str, cursor: usize) -> Option<Editing<'_>> {
    let cursor = cursor.min(text.len());
    let term = terms(text)
        .into_iter()
        .find(|term| term.range.start < cursor && cursor <= term.range.end);
    let Some(term) = term else {
        return Some(Editing::Key {
            prefix: "",
            range: cursor..cursor,
        });
    };
    match term.kind {
        Some(kind) if cursor >= term.value_start => {
            // Past a closing quote the value is done.
            let typed = text.get(term.value_start..cursor)?;
            if typed.len() > 1 && typed.starts_with('"') && typed.ends_with('"') {
                return None;
            }
            Some(Editing::Value {
                kind,
                value: typed.trim_start_matches('"'),
                range: term.range,
            })
        }
        Some(_) => None,
        None if term.value.contains(':') => None,
        None => Some(Editing::Key {
            prefix: text.get(term.range.start..cursor)?,
            range: term.range,
        }),
    }
}

/// How an accepted value is written back: quoted if it has a space.
pub(super) fn filter_text(kind: FilterKind, value: &str) -> String {
    if value.contains(char::is_whitespace) {
        format!("{}:\"{value}\" ", kind.key())
    } else {
        format!("{}:{value} ", kind.key())
    }
}

/// A query broken down, names still to be resolved.
#[derive(Default, PartialEq, Eq, Debug)]
pub(super) struct ParsedQuery {
    pub content: String,
    pub from: Vec<String>,
    pub mentions: Vec<String>,
    pub channels: Vec<String>,
    pub has: Vec<SearchHas>,
    pub author_types: Vec<AuthorType>,
    pub min_id: Option<Id<MessageMarker>>,
    pub max_id: Option<Id<MessageMarker>>,
    pub pinned: Option<bool>,
}

/// Fails on a value that can't mean anything, naming it for the user.
pub(super) fn parse(text: &str, today: NaiveDate) -> Result<ParsedQuery, String> {
    let mut parsed = ParsedQuery::default();
    let mut words = Vec::new();
    for term in terms(text) {
        let Some(kind) = term.kind else {
            words.push(term.value);
            continue;
        };
        let value = term.value.trim();
        if value.is_empty() {
            continue;
        }
        match kind {
            FilterKind::From => parsed.from.push(value.to_string()),
            FilterKind::Mentions => parsed.mentions.push(value.to_string()),
            FilterKind::In => parsed
                .channels
                .push(value.trim_start_matches('#').to_string()),
            FilterKind::Has => parsed.has.push(
                SearchHas::from_keyword(value)
                    .ok_or_else(|| format!("“{value}” isn't something a message can have."))?,
            ),
            FilterKind::AuthorType => parsed.author_types.push(
                AuthorType::from_keyword(value)
                    .ok_or_else(|| format!("“{value}” isn't a kind of author."))?,
            ),
            FilterKind::Pinned => {
                parsed.pinned = Some(match value.to_ascii_lowercase().as_str() {
                    "true" | "yes" => true,
                    "false" | "no" => false,
                    _ => return Err(format!("pinned: takes true or false, not “{value}”.")),
                })
            }
            FilterKind::Before | FilterKind::During | FilterKind::After => {
                let day = parse_date(value, today)
                    .ok_or_else(|| format!("“{value}” isn't a date. Try YYYY-MM-DD."))?;
                let start = day_start(day);
                let end = day.checked_add_days(Days::new(1)).and_then(day_start);
                match kind {
                    FilterKind::Before => parsed.max_id = narrower(parsed.max_id, start, true),
                    FilterKind::After => {
                        parsed.min_id =
                            narrower(parsed.min_id, end.map(|id| id.saturating_sub(1)), false)
                    }
                    _ => {
                        parsed.min_id =
                            narrower(parsed.min_id, start.map(|id| id.saturating_sub(1)), false);
                        parsed.max_id = narrower(parsed.max_id, end, true);
                    }
                }
            }
        }
    }
    parsed.content = words.join(" ");
    Ok(parsed)
}

/// Two bounds of the same side keep the tighter.
fn narrower(
    current: Option<Id<MessageMarker>>,
    new: Option<u64>,
    upper: bool,
) -> Option<Id<MessageMarker>> {
    let new = new.and_then(Id::new_checked);
    match (current, new) {
        (Some(current), Some(new)) => Some(if upper == (new < current) {
            new
        } else {
            current
        }),
        (current, new) => new.or(current),
    }
}

/// `YYYY-MM-DD`, or `today` and `yesterday`.
pub(super) fn parse_date(value: &str, today: NaiveDate) -> Option<NaiveDate> {
    match value.to_ascii_lowercase().as_str() {
        "today" => Some(today),
        "yesterday" => today.pred_opt(),
        _ => NaiveDate::parse_from_str(value, "%Y-%m-%d").ok(),
    }
}

/// The snowflake for local midnight starting `day`; `None` before Discord's epoch.
fn day_start(day: NaiveDate) -> Option<u64> {
    let midnight = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    let ms = u64::try_from(midnight.timestamp_millis()).ok()?;
    let since_epoch = ms.checked_sub(DISCORD_EPOCH_MS)?;
    Some(since_epoch << 22)
}

/// The results panel beside the conversation, for one search.
pub(super) struct SearchPanel {
    pub scope: SearchScope,
    pub filters: SearchFilters,
    /// Zero-based.
    pub page: usize,
    pub total: usize,
    pub hits: Vec<SearchHit>,
    /// Rows are of varied height, as pins are.
    pub list: ListState,
    pub loading: bool,
    /// Discord answered that it's still indexing; it's asked again shortly.
    pub indexing: bool,
    pub error: Option<String>,
    /// Dropped with the panel, so a page in flight can't land in a new search.
    pub _task: Option<Task<()>>,
}

impl SearchPanel {
    /// Capped where Discord stops taking offsets.
    pub fn page_count(&self) -> usize {
        let reachable = self.total.min(SEARCH_MAX_OFFSET + SEARCH_PAGE_SIZE);
        reachable.div_ceil(SEARCH_PAGE_SIZE).max(1)
    }
}

#[derive(Clone)]
pub(super) enum SearchSuggestion {
    Key(FilterKind),
    /// A user or channel, as the composer's autocomplete offers them.
    Name(FilterKind, Suggestion),
    /// A fixed value: a `has:` kind, a date, `true`.
    Value {
        kind: FilterKind,
        value: String,
        detail: Option<&'static str>,
    },
}

impl SearchSuggestion {
    pub fn key(&self) -> String {
        match self {
            Self::Key(kind) => kind.key().to_string(),
            Self::Name(kind, suggestion) => format!("{}-{}", kind.key(), suggestion.key()),
            Self::Value { kind, value, .. } => format!("{}-{value}", kind.key()),
        }
    }
}

/// Names picked from suggestions, lowercased, so the query can stay readable
/// and still resolve to whom or what was picked.
#[derive(Default)]
pub(super) struct SearchNames {
    pub users: HashMap<String, Id<UserMarker>>,
    pub channels: HashMap<String, Id<ChannelMarker>>,
}

/// The popout under the search box while it has focus.
pub(super) struct SearchSuggestions {
    /// The term an accepted suggestion replaces.
    pub range: Range<usize>,
    /// Set by `Editing::Value`, to title the list.
    pub kind: Option<FilterKind>,
    pub items: Vec<SearchSuggestion>,
    /// `None` leaves enter to run the search.
    pub selected: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
    }

    #[test]
    fn splits_filters_from_words() {
        let parsed = parse("hello from:alice has:image world in:#general", today()).unwrap();
        assert_eq!(parsed.content, "hello world");
        assert_eq!(parsed.from, ["alice"]);
        assert_eq!(parsed.has, [SearchHas::Image]);
        assert_eq!(parsed.channels, ["general"]);
    }

    #[test]
    fn keys_are_case_insensitive_and_names_can_be_quoted() {
        let parsed = parse("FROM:\"Big Bob\" authortype:bot", today()).unwrap();
        assert_eq!(parsed.from, ["Big Bob"]);
        assert_eq!(parsed.author_types, [AuthorType::Bot]);
    }

    #[test]
    fn rejects_unknown_values() {
        assert!(parse("has:cake", today()).is_err());
        assert!(parse("before:someday", today()).is_err());
        assert!(parse("pinned:maybe", today()).is_err());
    }

    #[test]
    fn a_colon_in_a_word_is_just_text() {
        let parsed = parse("ratio 16:9 https://x.y", today()).unwrap();
        assert_eq!(parsed.content, "ratio 16:9 https://x.y");
    }

    #[test]
    fn during_bounds_both_sides() {
        let parsed = parse("during:2026-01-02", today()).unwrap();
        let (min, max) = (parsed.min_id.unwrap(), parsed.max_id.unwrap());
        assert!(min < max);
        // About a day apart, give or take a DST shift.
        let span_ms = (max.get() - min.get()) >> 22;
        assert!((23 * 3_600_000..=25 * 3_600_000).contains(&span_ms));
    }

    #[test]
    fn finds_what_is_being_typed() {
        assert_eq!(
            editing_at("", 0),
            Some(Editing::Key {
                prefix: "",
                range: 0..0
            })
        );
        assert_eq!(
            editing_at("hi fr", 5),
            Some(Editing::Key {
                prefix: "fr",
                range: 3..5
            })
        );
        assert_eq!(
            editing_at("hi from:al", 10),
            Some(Editing::Value {
                kind: FilterKind::From,
                value: "al",
                range: 3..10
            })
        );
        assert_eq!(
            editing_at("from:\"Big B", 11),
            Some(Editing::Value {
                kind: FilterKind::From,
                value: "Big B",
                range: 0..11
            })
        );
        assert_eq!(editing_at("from:\"Big Bob\"", 14), None);
        assert_eq!(editing_at("16:9", 4), None);
    }

    #[test]
    fn quotes_values_with_spaces() {
        assert_eq!(filter_text(FilterKind::From, "bob"), "from:bob ");
        assert_eq!(
            filter_text(FilterKind::From, "Big Bob"),
            "from:\"Big Bob\" "
        );
    }
}
