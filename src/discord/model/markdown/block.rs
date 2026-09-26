//! The line-level rules: quotes, headings, subtext, lists and code blocks.
//! Everything between them is a paragraph, handed to [`super::inline`].

use super::inline;
use super::{Block, List, ListItem};

pub(super) fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    parse_into(source, false, &mut blocks);
    blocks
}

/// Parses `source` as a run of blocks. Inside a quote, quote markers are just
/// text, since quotes don't nest.
fn parse_into(source: &str, in_quote: bool, blocks: &mut Vec<Block>) {
    let mut paragraph = String::new();
    // Always sits at the start of a line, except just after a code block that
    // closed mid-line.
    let mut rest = source;

    while !rest.is_empty() {
        if !in_quote {
            // `>>> ` quotes everything after it, to the end of the text.
            if let Some(body) = rest
                .strip_prefix(">>> ")
                .or_else(|| rest.strip_prefix(">>>\n"))
            {
                flush(&mut paragraph, blocks);
                let mut inner = Vec::new();
                parse_into(body, true, &mut inner);
                blocks.push(Block::Quote(inner));
                return;
            }
            if quote_line(rest).is_some() {
                flush(&mut paragraph, blocks);
                let mut body = String::new();
                while let Some(line) = quote_line(rest) {
                    let (_, next) = split_line(rest);
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(line);
                    rest = next;
                }
                let mut inner = Vec::new();
                parse_into(&body, true, &mut inner);
                blocks.push(Block::Quote(inner));
                continue;
            }
        }

        let (line, next) = split_line(rest);
        if let Some((level, content)) = heading(line) {
            flush(&mut paragraph, blocks);
            blocks.push(Block::Heading {
                level,
                content: inline::parse(content, true),
            });
            rest = next;
            continue;
        }
        if let Some(content) = subtext(line) {
            flush(&mut paragraph, blocks);
            blocks.push(Block::Subtext(inline::parse(content, true)));
            rest = next;
            continue;
        }
        if list_item(line).is_some() {
            flush(&mut paragraph, blocks);
            rest = parse_lists(rest, blocks);
            continue;
        }

        // A code block can open anywhere on a line, and runs to its closing
        // fence however many lines on.
        if let Some((start, block, len)) = find_code_block(rest, line.len()) {
            paragraph.push_str(&rest[..start]);
            flush(&mut paragraph, blocks);
            blocks.push(block);
            rest = &rest[start + len..];
            // The block ends its own line; a newline straight after it would
            // otherwise draw as an empty one.
            rest = rest.strip_prefix('\n').unwrap_or(rest);
            continue;
        }

        paragraph.push_str(line);
        if next.len() < rest.len() - line.len() {
            paragraph.push('\n');
        }
        rest = next;
    }
    flush(&mut paragraph, blocks);
}

/// Ends the paragraph being gathered. The newline that led into the next block
/// is dropped — the block starts its own line anyway — but any blank lines
/// before it are kept, as Discord keeps them.
fn flush(paragraph: &mut String, blocks: &mut Vec<Block>) {
    let text = paragraph.strip_suffix('\n').unwrap_or(paragraph);
    if !text.is_empty() {
        blocks.push(Block::Paragraph(inline::parse(text, true)));
    }
    paragraph.clear();
}

/// The first line of `text`, and everything after its newline.
fn split_line(text: &str) -> (&str, &str) {
    match text.find('\n') {
        Some(end) => (&text[..end], &text[end + 1..]),
        None => (text, ""),
    }
}

/// A `> ` line's text, without the marker. A bare `>` isn't a quote.
fn quote_line(text: &str) -> Option<&str> {
    let (line, _) = split_line(text);
    if line.starts_with(">>> ") {
        return None;
    }
    line.strip_prefix("> ")
}

/// `#` to `###`, a space, then something to show.
fn heading(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let content = line[level..].strip_prefix(' ')?.trim();
    (!content.is_empty()).then_some((level as u8, content))
}

fn subtext(line: &str) -> Option<&str> {
    let content = line.strip_prefix("-# ")?.trim();
    (!content.is_empty()).then_some(content)
}

/// One line of a list: its indent, its number (`None` for a bullet), and its
/// text.
struct ItemLine<'a> {
    indent: usize,
    number: Option<u64>,
    content: &'a str,
}

fn list_item(line: &str) -> Option<ItemLine<'_>> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    let rest = &line[indent..];
    let (number, after) = if let Some(after) = rest.strip_prefix(['-', '*']) {
        (None, after)
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = rest[digits..].strip_prefix('.')?;
        (Some(rest[..digits].parse().ok()?), after)
    };
    if !after.starts_with(' ') {
        return None;
    }
    let content = after.trim();
    (!content.is_empty()).then_some(ItemLine {
        indent,
        number,
        content,
    })
}

/// Parses the run of list lines at the start of `text` into one list or more
/// — switching between bullets and numbers starts a new one — and returns what
/// follows the run.
fn parse_lists<'a>(mut text: &'a str, blocks: &mut Vec<Block>) -> &'a str {
    let mut lines = Vec::new();
    while !text.is_empty() {
        let (line, next) = split_line(text);
        let Some(item) = list_item(line) else {
            break;
        };
        lines.push(item);
        text = next;
    }

    let mut cursor = 0;
    while cursor < lines.len() {
        let indent = lines[cursor].indent;
        blocks.push(Block::List(build_list(&lines, &mut cursor, indent)));
    }
    text
}

/// Builds one list from `lines[*cursor..]` at `indent`, taking deeper lines
/// as nested lists under the item above them. Stops at a shallower line, or a
/// line of the other kind at the same depth.
fn build_list(lines: &[ItemLine], cursor: &mut usize, indent: usize) -> List {
    let ordered = lines[*cursor].number.is_some();
    let mut list = List {
        start: lines[*cursor].number,
        items: Vec::new(),
    };
    while let Some(line) = lines.get(*cursor) {
        if line.indent < indent {
            break;
        }
        if line.indent > indent
            && let Some(parent) = list.items.last_mut()
        {
            let nested = build_list(lines, cursor, line.indent);
            parent.children.push(nested);
            continue;
        }
        if line.number.is_some() != ordered && !list.items.is_empty() {
            break;
        }
        list.items.push(ListItem {
            content: inline::parse(line.content, true),
            children: Vec::new(),
        });
        *cursor += 1;
    }
    list
}

/// Finds a code block opening within the first `line_len` bytes of `text`:
/// where it starts, the block, and how many bytes it spans. An opening fence
/// with no closing one is just text.
fn find_code_block(text: &str, line_len: usize) -> Option<(usize, Block, usize)> {
    let mut from = 0;
    while let Some(offset) = text[from..line_len].find("```") {
        let start = from + offset;
        let escaped = text[..start].ends_with('\\');
        if !escaped && let Some((block, len)) = code_block(&text[start..]) {
            return Some((start, block, len));
        }
        from = start + 1;
        if from >= line_len {
            break;
        }
    }
    None
}

/// Parses a code block at the start of `text`, which begins with its fence.
/// The word right after the fence is the language only when a newline follows
/// it and there is code after that; leading and trailing blank lines are
/// dropped.
fn code_block(text: &str) -> Option<(Block, usize)> {
    let body = &text[3..];
    let close = body.find("```")?;
    let inner = &body[..close];

    let (language, code) = match inner.split_once('\n') {
        Some((first, code)) if is_language(first) && code.chars().any(|c| c != '\n') => {
            (Some(first.to_string()), code)
        }
        _ => (None, inner),
    };
    let code = code.trim_matches('\n');
    if code.is_empty() {
        return None;
    }
    Some((
        Block::Code {
            language,
            code: code.to_string(),
        },
        3 + close + 3,
    ))
}

fn is_language(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.' | '#'))
}
