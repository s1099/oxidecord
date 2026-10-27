//! Inline rules: delimiters, code spans, links, mentions, emoji, timestamps.
//!
//! Discord tries rules in a fixed order at each position; where delimiter rules
//! overlap (`*`/`**`, `_`/`__`) the longest match wins. The matchers mirror its
//! regexes by hand since the `regex` crate has no lookaround.

use std::sync::LazyLock;

use regex::Regex;
use twilight_model::id::Id;

use super::{CustomEmoji, GuildNavigation, Inline, Mention, Timestamp, TimestampStyle};

static USER_MENTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<@!?(\d+)>").unwrap());
static ROLE_MENTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<@&(\d+)>").unwrap());
static CHANNEL_MENTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<#(\d+)>").unwrap());
static EMOJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<(a?):(\w+):(\d+)>").unwrap());
static TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<t:(-?\d{1,17})(?::([tTdDfFR]))?>").unwrap());
static COMMAND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^</([^:<>/][^:<>]*):(\d+)>").unwrap());
static GUILD_NAVIGATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<id:(customize|browse|guide|linked-roles)>").unwrap());
static AUTOLINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<(https?://[^\s>]+)>").unwrap());

/// Without `links`, URLs stay plain text, as in a masked link's label and in
/// embed titles and field names.
pub(super) fn parse(source: &str, links: bool) -> Vec<Inline> {
    let mut parser = Parser {
        source,
        links,
        out: Vec::new(),
        text: String::new(),
    };
    parser.run();
    parser.finish()
}

struct Parser<'a> {
    source: &'a str,
    links: bool,
    out: Vec<Inline>,
    text: String,
}

#[derive(Clone, Copy)]
enum Delimited {
    Italic,
    Bold,
    Underline,
    Strikethrough,
    Spoiler,
}

impl Parser<'_> {
    fn run(&mut self) {
        let source = self.source;
        let mut pos = 0;
        while pos < source.len() {
            let rest = &source[pos..];
            let c = rest.chars().next().expect("not at the end");
            let consumed = match c {
                '\\' => self.escape(rest),
                '`' => self.code(rest),
                '<' => self.angle(rest),
                '@' => self.everyone(rest),
                '[' if self.links => self.masked_link(rest),
                'h' if self.links && !prev_is_alphanumeric(source, pos) => self.url(rest),
                '*' | '_' | '~' | '|' => self.delimited(source, pos),
                _ => None,
            };
            match consumed {
                Some(len) => pos += len,
                None => {
                    self.text.push(c);
                    pos += c.len_utf8();
                }
            }
        }
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush();
        self.out
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.out.push(Inline::Text(std::mem::take(&mut self.text)));
        }
    }

    fn push(&mut self, node: Inline) {
        self.flush();
        self.out.push(node);
    }

    fn escape(&mut self, rest: &str) -> Option<usize> {
        let next = rest[1..].chars().next()?;
        if next.is_alphanumeric() || next.is_whitespace() {
            return None;
        }
        self.text.push(next);
        Some(1 + next.len_utf8())
    }

    /// Closes on a backtick run of the same length; nothing inside is parsed.
    fn code(&mut self, rest: &str) -> Option<usize> {
        let ticks = rest.bytes().take_while(|&b| b == b'`').count();
        let mut search = ticks;
        let result = loop {
            let Some(offset) = rest[search..].find('`') else {
                break None;
            };
            let start = search + offset;
            let run = rest[start..].bytes().take_while(|&b| b == b'`').count();
            if run == ticks && start > ticks {
                break Some(start);
            }
            search = start + run;
        };
        let Some(close) = result else {
            // Consume the whole run so the next backtick doesn't open a span.
            self.text.push_str(&rest[..ticks]);
            return Some(ticks);
        };
        let code = rest[ticks..close].trim();
        if code.is_empty() {
            self.text.push_str(&rest[..close + ticks]);
        } else {
            self.push(Inline::Code(code.to_string()));
        }
        Some(close + ticks)
    }

    fn angle(&mut self, rest: &str) -> Option<usize> {
        if let Some(caps) = USER_MENTION.captures(rest) {
            let id = Id::new_checked(caps[1].parse().ok()?)?;
            self.push(Inline::Mention(Mention::User(id)));
            return Some(caps[0].len());
        }
        if let Some(caps) = ROLE_MENTION.captures(rest) {
            let id = Id::new_checked(caps[1].parse().ok()?)?;
            self.push(Inline::Mention(Mention::Role(id)));
            return Some(caps[0].len());
        }
        if let Some(caps) = CHANNEL_MENTION.captures(rest) {
            let id = Id::new_checked(caps[1].parse().ok()?)?;
            self.push(Inline::Mention(Mention::Channel(id)));
            return Some(caps[0].len());
        }
        if let Some(caps) = EMOJI.captures(rest) {
            let id = Id::new_checked(caps[3].parse().ok()?)?;
            self.push(Inline::Emoji(CustomEmoji {
                id,
                name: caps[2].to_string(),
                animated: !caps[1].is_empty(),
            }));
            return Some(caps[0].len());
        }
        if let Some(caps) = TIMESTAMP.captures(rest) {
            let unix = caps[1].parse().ok()?;
            let style = match caps.get(2).map(|m| m.as_str()) {
                Some("t") => TimestampStyle::ShortTime,
                Some("T") => TimestampStyle::LongTime,
                Some("d") => TimestampStyle::ShortDate,
                Some("D") => TimestampStyle::LongDate,
                Some("F") => TimestampStyle::LongDateTime,
                Some("R") => TimestampStyle::Relative,
                _ => TimestampStyle::ShortDateTime,
            };
            self.push(Inline::Timestamp(Timestamp { unix, style }));
            return Some(caps[0].len());
        }
        if let Some(caps) = COMMAND.captures(rest) {
            self.push(Inline::Mention(Mention::Command(caps[1].to_string())));
            return Some(caps[0].len());
        }
        if let Some(caps) = GUILD_NAVIGATION.captures(rest) {
            let target = match &caps[1] {
                "customize" => GuildNavigation::Customize,
                "browse" => GuildNavigation::Browse,
                "guide" => GuildNavigation::Guide,
                _ => GuildNavigation::LinkedRoles,
            };
            self.push(Inline::Mention(Mention::GuildNavigation(target)));
            return Some(caps[0].len());
        }
        if self.links
            && let Some(caps) = AUTOLINK.captures(rest)
        {
            self.push(Inline::Link {
                url: caps[1].to_string(),
                label: None,
            });
            return Some(caps[0].len());
        }
        None
    }

    fn everyone(&mut self, rest: &str) -> Option<usize> {
        let (mention, len) = if rest.starts_with("@everyone") {
            (Mention::Everyone, "@everyone".len())
        } else if rest.starts_with("@here") {
            (Mention::Here, "@here".len())
        } else {
            return None;
        };
        self.push(Inline::Mention(mention));
        Some(len)
    }

    /// `[label](https://url)`, the URL optionally in angle brackets, followed by
    /// an ignored quoted title.
    fn masked_link(&mut self, rest: &str) -> Option<usize> {
        let mut depth = 0;
        let mut label_end = None;
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        label_end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let label_end = label_end?;
        let label = &rest[1..label_end];
        if label.trim().is_empty() {
            return None;
        }

        let target = rest[label_end + 1..].strip_prefix('(')?;
        let trimmed = target.trim_start();
        let mut pos = target.len() - trimmed.len();
        let bracketed = trimmed.starts_with('<');
        if bracketed {
            pos += 1;
        }
        let url_text = &target[pos..];
        if !url_text.starts_with("http://") && !url_text.starts_with("https://") {
            return None;
        }
        // Parens that balance inside the URL stay (Wikipedia's, say).
        let mut parens = 0;
        let mut url_len = url_text.len();
        for (i, c) in url_text.char_indices() {
            let ends = match c {
                '(' => {
                    parens += 1;
                    false
                }
                ')' if parens == 0 => true,
                ')' => {
                    parens -= 1;
                    false
                }
                '>' => bracketed,
                c => c.is_whitespace(),
            };
            if ends {
                url_len = i;
                break;
            }
        }
        let url = &url_text[..url_len];
        pos += url_len;
        if bracketed {
            pos += target[pos..].strip_prefix('>').map(|_| 1)?;
        }
        let after = &target[pos..];
        let after_trimmed = after.trim_start();
        pos += after.len() - after_trimmed.len();
        if let Some(quoted) = after_trimmed.strip_prefix('"') {
            let close = quoted.find('"')?;
            pos += close + 2;
            let after = &target[pos..];
            pos += after.len() - after.trim_start().len();
        }
        target[pos..].strip_prefix(')')?;

        self.push(Inline::Link {
            url: url.to_string(),
            label: Some(parse(label, false)),
        });
        Some(label_end + 2 + pos + 1)
    }

    /// Trailing punctuation that more likely ends the sentence is left out, as
    /// is an unbalanced closing paren.
    fn url(&mut self, rest: &str) -> Option<usize> {
        if !rest.starts_with("http://") && !rest.starts_with("https://") {
            return None;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '<')
            .unwrap_or(rest.len());
        let mut url = &rest[..end];
        while let Some(last) = url.chars().last() {
            let trim = match last {
                '.' | ',' | ':' | ';' | '"' | '\'' | '!' | '?' | ']' | '*' | '_' | '~' | '|' => {
                    true
                }
                ')' => url.matches('(').count() < url.matches(')').count(),
                _ => false,
            };
            if !trim {
                break;
            }
            url = &url[..url.len() - last.len_utf8()];
        }
        if url.len() <= "https://".len() {
            return None;
        }
        self.push(Inline::Link {
            url: url.to_string(),
            label: None,
        });
        Some(url.len())
    }

    /// The longer match wins and a tie goes to italic, which is how `***both***`
    /// becomes italic around bold.
    fn delimited(&mut self, source: &str, pos: usize) -> Option<usize> {
        let rest = &source[pos..];
        let candidates: [Option<(Delimited, usize, usize)>; 2] = match rest.as_bytes()[0] {
            b'*' => [
                italic_star(rest).map(|(inner, len)| (Delimited::Italic, inner, len)),
                wrapped(rest, "**", Some('*')).map(|(inner, len)| (Delimited::Bold, inner, len)),
            ],
            b'_' => [
                (!prev_is_word(source, pos))
                    .then(|| italic_underscore(rest))
                    .flatten()
                    .map(|(inner, len)| (Delimited::Italic, inner, len)),
                wrapped(rest, "__", Some('_'))
                    .map(|(inner, len)| (Delimited::Underline, inner, len)),
            ],
            b'~' => [
                wrapped(rest, "~~", None)
                    .map(|(inner, len)| (Delimited::Strikethrough, inner, len)),
                None,
            ],
            b'|' => [
                wrapped(rest, "||", None).map(|(inner, len)| (Delimited::Spoiler, inner, len)),
                None,
            ],
            _ => return None,
        };
        let (kind, inner_len, len) = candidates
            .into_iter()
            .flatten()
            .reduce(|best, next| if next.2 > best.2 { next } else { best })?;

        let delimiter = (len - inner_len) / 2;
        let children = parse(&rest[delimiter..delimiter + inner_len], self.links);
        self.push(match kind {
            Delimited::Italic => Inline::Italic(children),
            Delimited::Bold => Inline::Bold(children),
            Delimited::Underline => Inline::Underline(children),
            Delimited::Strikethrough => Inline::Strikethrough(children),
            Delimited::Spoiler => Inline::Spoiler(children),
        });
        Some(len)
    }
}

/// The shortest non-empty run to the next unescaped closing delimiter, which
/// mustn't be followed by `forbid_after`. Returns inner and whole lengths.
fn wrapped(rest: &str, delimiter: &str, forbid_after: Option<char>) -> Option<(usize, usize)> {
    let open = delimiter.len();
    let body = rest.strip_prefix(delimiter)?;
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            chars.next();
            continue;
        }
        if i > 0 && body[i..].starts_with(delimiter) {
            let after = body[i + delimiter.len()..].chars().next();
            if forbid_after.is_none() || after != forbid_after {
                return Some((i, i + open * 2));
            }
        }
    }
    None
}

/// `*italic*`: must start on a non-space, can't end on one, and can hold `**`
/// but no lone `*`.
fn italic_star(rest: &str) -> Option<(usize, usize)> {
    let body = rest.strip_prefix('*')?;
    if body.chars().next().is_none_or(char::is_whitespace) {
        return None;
    }
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut consumed = false;
    loop {
        if consumed && bytes.get(i) == Some(&b'*') && bytes.get(i + 1) != Some(&b'*') {
            return Some((i, i + 2));
        }
        let rest = body.get(i..)?;
        let c = rest.chars().next()?;
        if rest.starts_with("**") {
            i += 2;
        } else if c == '\\' {
            i += 1 + rest[1..].chars().next()?.len_utf8();
        } else if c.is_whitespace() {
            // Whitespace only counts if something other than the closing `*` follows.
            let spaces = rest.len() - rest.trim_start().len();
            let after = &rest[spaces..];
            let next = after.chars().next()?;
            if next == '*' && !after.starts_with("**") {
                return None;
            }
            i += spaces;
            continue;
        } else if c == '*' {
            return None;
        } else {
            i += c.len_utf8();
        }
        consumed = true;
    }
}

/// `_italic_`: can hold `__` but no lone `_`, and the closing `_` can't run
/// into a word.
fn italic_underscore(rest: &str) -> Option<(usize, usize)> {
    let body = rest.strip_prefix('_')?;
    let mut chars = body.char_indices().peekable();
    let mut consumed = false;
    while let Some((i, c)) = chars.next() {
        match c {
            '_' if body[i..].starts_with("__") => {
                chars.next();
            }
            '_' => {
                let next = body[i + 1..].chars().next();
                if consumed && !next.is_some_and(is_word_char) {
                    return Some((i, i + 2));
                }
                return None;
            }
            '\\' => {
                chars.next();
            }
            _ => {}
        }
        consumed = true;
    }
    None
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn prev_is_word(source: &str, pos: usize) -> bool {
    source[..pos].chars().next_back().is_some_and(is_word_char)
}

fn prev_is_alphanumeric(source: &str, pos: usize) -> bool {
    source[..pos]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
}
