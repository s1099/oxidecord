//! The emoji picker's contents, and turning the `:name:` it inserts for a
//! custom emoji into the `<:name:id>` Discord renders.
//!
//! Which custom emoji the user may send is decided by the caller; this module
//! only lays out and searches what it's handed.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::discord::GuildEmoji;

/// Emoji per picker row.
pub(in crate::screens::home) const COLUMNS: usize = 9;

/// The newest emoji version Windows' Segoe UI Emoji draws. Anything later would
/// show as tofu, so it's left out of the picker rather than offered broken.
const NEWEST_EMOJI: emojis::EmojiVersion = emojis::EmojiVersion::new(15, 0);

/// One cell in the picker.
#[derive(Clone)]
pub(in crate::screens::home) enum PickerEmoji {
    Unicode(&'static emojis::Emoji),
    Custom {
        emoji: GuildEmoji,
        /// Why it can't be sent here, for one that's shown but locked.
        locked: Option<&'static str>,
    },
}

impl PickerEmoji {
    /// The `:name:` shown under the picker for the hovered emoji.
    pub fn label(&self) -> String {
        match self {
            Self::Unicode(emoji) => format!(":{}:", emoji.shortcode().unwrap_or(emoji.name())),
            Self::Custom { emoji, .. } => format!(":{}:", emoji.name),
        }
    }
}

/// A titled run of emoji: one guild's, or one unicode category.
pub(in crate::screens::home) struct PickerSection {
    pub title: String,
    pub emojis: Vec<PickerEmoji>,
}

/// A line of the picker's list. Headers take a whole line so the list stays
/// uniform in height and can be virtualized.
pub(in crate::screens::home) enum PickerRow {
    Header(String),
    Emojis(Vec<PickerEmoji>),
}

/// The unicode categories, filtered to what the system font can draw. Built
/// once: the emoji crate's table is static, so this never changes.
static UNICODE: LazyLock<Vec<(&'static str, Vec<&'static emojis::Emoji>)>> = LazyLock::new(|| {
    emojis::Group::iter()
        .map(|group| {
            let emojis = group
                .emojis()
                .filter(|emoji| emoji.emoji_version() <= NEWEST_EMOJI)
                .collect();
            (group_title(group), emojis)
        })
        .collect()
});

fn group_title(group: emojis::Group) -> &'static str {
    use emojis::Group;
    match group {
        Group::SmileysAndEmotion => "Smileys & Emotion",
        Group::PeopleAndBody => "People & Body",
        Group::AnimalsAndNature => "Animals & Nature",
        Group::FoodAndDrink => "Food & Drink",
        Group::TravelAndPlaces => "Travel & Places",
        Group::Activities => "Activities",
        Group::Objects => "Objects",
        Group::Symbols => "Symbols",
        Group::Flags => "Flags",
    }
}

/// The picker's lines: the custom sections handed in, then every unicode
/// category, all narrowed to `query` (matched against names and shortcodes,
/// ignoring case). Sections left empty by the search are dropped.
pub(in crate::screens::home) fn picker_rows(
    custom: Vec<PickerSection>,
    query: &str,
) -> Vec<PickerRow> {
    let query = query.trim().to_lowercase();
    let unicode = UNICODE.iter().map(|(title, emojis)| PickerSection {
        title: title.to_string(),
        emojis: emojis
            .iter()
            .filter(|emoji| unicode_matches(emoji, &query))
            .map(|&emoji| PickerEmoji::Unicode(emoji))
            .collect(),
    });

    let mut rows = Vec::new();
    for mut section in custom.into_iter().chain(unicode) {
        if !query.is_empty() {
            section.emojis.retain(|emoji| match emoji {
                PickerEmoji::Custom { emoji, .. } => emoji.name.to_lowercase().contains(&query),
                // Already filtered above.
                PickerEmoji::Unicode(_) => true,
            });
        }
        if section.emojis.is_empty() {
            continue;
        }
        rows.push(PickerRow::Header(section.title));
        rows.extend(
            section
                .emojis
                .chunks(COLUMNS)
                .map(|chunk| PickerRow::Emojis(chunk.to_vec())),
        );
    }
    rows
}

fn unicode_matches(emoji: &emojis::Emoji, query: &str) -> bool {
    query.is_empty()
        || emoji.name().contains(query)
        || emoji.shortcodes().any(|code| code.contains(query))
}

/// Custom emoji the user may send here, by name, for [`resolve_custom_emoji`].
/// The first of a name wins, so callers list the open guild's own first.
pub(in crate::screens::home) fn emoji_names<'a>(
    usable: impl IntoIterator<Item = &'a GuildEmoji>,
) -> HashMap<&'a str, &'a GuildEmoji> {
    let mut names = HashMap::new();
    for emoji in usable {
        names.entry(emoji.name.as_str()).or_insert(emoji);
    }
    names
}

/// Replaces each `:name:` naming a custom emoji in `names` with the markup
/// Discord renders it from, which is what its own client does on send. Code
/// spans and blocks are left alone, as is markup that's already resolved.
pub(in crate::screens::home) fn resolve_custom_emoji(
    text: &str,
    names: &HashMap<&str, &GuildEmoji>,
) -> String {
    if names.is_empty() {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                // A run of n backticks is closed by the next run of exactly n.
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                i += run;
                if let Some(end) = closing_backticks(&bytes[i..], run) {
                    i += end + run;
                }
            }
            b'<' => {
                // `<:name:id>` and `<a:name:id>` are already emoji; skipping
                // them keeps their inner `:name:` from being resolved again.
                i += 1 + existing_markup_len(&bytes[i + 1..]).unwrap_or(0);
            }
            b':' => {
                let len = bytes[i + 1..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
                    .count();
                let close = i + 1 + len;
                let emoji = (len >= 2 && bytes.get(close) == Some(&b':'))
                    .then(|| names.get(&text[i + 1..close]))
                    .flatten();
                match emoji {
                    Some(emoji) => {
                        out.push_str(&text[copied..i]);
                        out.push_str(&emoji.markdown());
                        i = close + 1;
                        copied = i;
                    }
                    // The colon may yet open a name that follows it.
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Where a run of exactly `run` backticks starts in `bytes`.
fn closing_backticks(bytes: &[u8], run: usize) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            if len == run {
                return Some(i);
            }
            i += len;
        } else {
            i += 1;
        }
    }
    None
}

/// The length of `a:name:id>` or `:name:id>` at the start of `bytes`.
fn existing_markup_len(bytes: &[u8]) -> Option<usize> {
    let mut i = usize::from(bytes.first() == Some(&b'a'));
    (bytes.get(i) == Some(&b':')).then_some(())?;
    i += 1;
    let name = bytes[i..]
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_')
        .count();
    i += name;
    (name > 0 && bytes.get(i) == Some(&b':')).then_some(())?;
    i += 1;
    let id = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    i += id;
    (id > 0 && bytes.get(i) == Some(&b'>')).then_some(i + 1)
}

#[cfg(test)]
mod tests {
    use twilight_model::id::Id;

    use super::*;

    fn emoji(id: u64, name: &str, animated: bool) -> GuildEmoji {
        GuildEmoji {
            id: Id::new(id),
            name: name.to_string(),
            animated,
            available: true,
            roles: Vec::new(),
        }
    }

    fn resolve(text: &str) -> String {
        let emojis = [emoji(1, "wave", false), emoji(2, "party", true)];
        resolve_custom_emoji(text, &emoji_names(&emojis))
    }

    #[test]
    fn resolves_known_names() {
        assert_eq!(resolve("hi :wave:"), "hi <:wave:1>");
        assert_eq!(resolve(":wave::party:"), "<:wave:1><a:party:2>");
        assert_eq!(resolve("a: :wave:"), "a: <:wave:1>");
    }

    #[test]
    fn leaves_unknown_names_and_code_alone() {
        assert_eq!(resolve(":nope: 12:30"), ":nope: 12:30");
        assert_eq!(resolve("`:wave:` :wave:"), "`:wave:` <:wave:1>");
        assert_eq!(resolve("```\n:wave:\n```"), "```\n:wave:\n```");
        assert_eq!(resolve("`` ` :wave: ``"), "`` ` :wave: ``");
    }

    #[test]
    fn leaves_existing_markup_alone() {
        assert_eq!(resolve("<:wave:99> :wave:"), "<:wave:99> <:wave:1>");
        assert_eq!(resolve("<a:wave:99>"), "<a:wave:99>");
    }

    #[test]
    fn first_name_wins() {
        let emojis = [emoji(1, "wave", false), emoji(2, "wave", false)];
        let names = emoji_names(&emojis);
        assert_eq!(resolve_custom_emoji(":wave:", &names), "<:wave:1>");
    }

    #[test]
    fn search_drops_empty_sections() {
        let custom = vec![PickerSection {
            title: "Guild".into(),
            emojis: vec![PickerEmoji::Custom {
                emoji: emoji(1, "wave", false),
                locked: None,
            }],
        }];
        let rows = picker_rows(custom, "WAVE");
        assert!(matches!(&rows[0], PickerRow::Header(title) if title == "Guild"));
        // The unicode waving hand matches too.
        assert!(rows.len() > 2);
        assert!(picker_rows(Vec::new(), "zzzzqqq").is_empty());
    }
}
