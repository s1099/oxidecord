//! Discord's flavour of markdown, parsed once when a message or embed arrives.
//!
//! Not CommonMark: line breaks are kept, `__` underlines, `||` is a spoiler,
//! `-#` is subtext, headings stop at three levels, quotes can't nest, and
//! `<...>` tokens stand in for mentions, emoji and timestamps. Follows
//! Discord's own parser (a simple-markdown fork) for everything people type.
//!
//! Mentions stay ids: who `<@123>` is depends on what's loaded when it's drawn.

mod block;
mod inline;
#[cfg(test)]
mod tests;

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, EmojiMarker, RoleMarker, UserMarker},
};

use super::cdn;
use super::time;

/// Discord's cap for drawing emoji-only text jumbo.
const JUMBO_MAX: usize = 30;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Markdown {
    pub blocks: Vec<Block>,
    /// Nothing but up to [`JUMBO_MAX`] emoji. Only honoured for message content.
    pub jumbo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    /// `-# `
    Subtext(Vec<Inline>),
    List(List),
    /// `> ` lines, or everything after `>>> `. Never holds another quote.
    Quote(Vec<Block>),
    Code {
        /// The word after the opening fence; nothing highlights it yet.
        language: Option<String>,
        code: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct List {
    /// `None` for bullets.
    pub start: Option<u64>,
    pub items: Vec<ListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub content: Vec<Inline>,
    pub children: Vec<List>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Underline(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Spoiler(Vec<Inline>),
    Code(String),
    Link {
        url: String,
        /// `None` for a bare URL, shown as itself.
        label: Option<Vec<Inline>>,
    },
    Mention(Mention),
    Emoji(CustomEmoji),
    Timestamp(Timestamp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mention {
    User(Id<UserMarker>),
    Role(Id<RoleMarker>),
    Channel(Id<ChannelMarker>),
    Everyone,
    Here,
    /// `</name:id>`, a clickable slash command.
    Command(String),
    /// `<id:customize>` and friends: links to a guild's own pages.
    GuildNavigation(GuildNavigation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuildNavigation {
    Customize,
    Browse,
    Guide,
    LinkedRoles,
}

impl GuildNavigation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Customize => "Channels & Roles",
            Self::Browse => "Browse Channels",
            Self::Guide => "Server Guide",
            Self::LinkedRoles => "Linked Roles",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomEmoji {
    pub id: Id<EmojiMarker>,
    pub name: String,
    pub animated: bool,
}

impl CustomEmoji {
    pub fn url(&self) -> String {
        cdn::emoji_url(self.id, self.animated)
    }
}

/// `<t:unix:style>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub unix: i64,
    pub style: TimestampStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimestampStyle {
    /// `t`: 4:20 PM
    ShortTime,
    /// `T`: 4:20:30 PM
    LongTime,
    /// `d`: 09/26/2026
    ShortDate,
    /// `D`: September 26, 2026
    LongDate,
    /// `f`, and the default: September 26, 2026 4:20 PM
    ShortDateTime,
    /// `F`: Saturday, September 26, 2026 4:20 PM
    LongDateTime,
    /// `R`: in 2 hours, 3 days ago
    Relative,
}

impl Timestamp {
    /// A relative style is measured from `now` (Unix seconds).
    pub fn display(&self, now: i64) -> String {
        match self.style {
            TimestampStyle::ShortTime => time::format_unix_time(self.unix, false),
            TimestampStyle::LongTime => time::format_unix_time(self.unix, true),
            TimestampStyle::ShortDate => time::format_unix_short_date(self.unix),
            TimestampStyle::LongDate => time::format_unix_date(self.unix, false),
            TimestampStyle::ShortDateTime => format!(
                "{} {}",
                time::format_unix_date(self.unix, false),
                time::format_unix_time(self.unix, false)
            ),
            TimestampStyle::LongDateTime => self.full(),
            TimestampStyle::Relative => time::format_relative(self.unix, now),
        }
    }

    pub fn full(&self) -> String {
        time::format_unix_full(self.unix)
    }
}

impl Markdown {
    pub fn parse(source: &str) -> Self {
        let blocks = block::parse(source);
        let jumbo = is_jumbo(&blocks);
        Self { blocks, jumbo }
    }
}

/// Inline formatting without links: an embed's title or a field's name.
pub fn parse_inline(source: &str) -> Vec<Inline> {
    inline::parse(source, false)
}

/// Inline-only with lines run together, for a reply's one-line quote.
pub fn parse_preview(source: &str) -> Vec<Inline> {
    /// Past this the preview is cut off on screen anyway.
    const MAX_CHARS: usize = 300;
    let flat: String = source
        .chars()
        .take(MAX_CHARS)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    inline::parse(&flat, true)
}

fn is_jumbo(blocks: &[Block]) -> bool {
    let [Block::Paragraph(inlines)] = blocks else {
        return false;
    };
    let mut count = 0;
    for inline in inlines {
        match inline {
            Inline::Emoji(_) => count += 1,
            Inline::Text(text) => match count_unicode_emoji(text) {
                Some(n) => count += n,
                None => return false,
            },
            _ => return false,
        }
    }
    (1..=JUMBO_MAX).contains(&count)
}

/// `None` if the text holds anything but emoji and whitespace. Approximates
/// the emoji grammar without its tables: joiners, variation selectors, skin
/// tones and tags ride along with their emoji, and a regional-indicator pair
/// is one flag.
fn count_unicode_emoji(text: &str) -> Option<usize> {
    let mut count = 0;
    let mut regional = 0;
    let mut joined = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = c as u32;
        if c.is_whitespace() {
            joined = false;
        } else if code == 0x200D {
            joined = true;
        } else if matches!(code, 0xFE0E | 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F) {
            // Modifies the emoji before it.
        } else if (0x1F1E6..=0x1F1FF).contains(&code) {
            regional += 1;
            if regional % 2 == 1 {
                count += 1;
            }
        } else if matches!(c, '0'..='9' | '#' | '*') {
            // Only a keycap: digit, optional variation selector, keycap mark.
            if chars.peek() == Some(&'\u{FE0F}') {
                chars.next();
            }
            if chars.next() != Some('\u{20E3}') {
                return None;
            }
            count += 1;
        } else if is_pictographic(code) {
            if !joined {
                count += 1;
            }
            joined = false;
        } else {
            return None;
        }
    }
    Some(count)
}

/// The blocks where emoji live, standing in for the Unicode property table.
fn is_pictographic(code: u32) -> bool {
    matches!(
        code,
        0x1F000..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x2300..=0x23FF
            | 0x2B00..=0x2BFF
            | 0x2190..=0x21FF
            | 0x25A0..=0x25FF
            | 0x2900..=0x297F
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0xA9
            | 0xAE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x24C2
    )
}
