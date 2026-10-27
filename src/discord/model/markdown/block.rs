//! Line-level rules: quotes, headings, subtext, lists and code blocks.
//! Everything else is a paragraph, handed to [`super::inline`].

use super::inline;
use super::{Block, List, ListItem};

pub(super) fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    parse_into(source, false, &mut blocks);
    blocks
}

/// Inside a quote, quote markers are plain text since quotes don't nest.
fn parse_into(source: &str, in_quote: bool, blocks: &mut Vec<Block>) {
    let mut paragraph = String::new();
    // At the start of a line, except just after a code block closed mid-line.
    let mut rest = source;

    while !rest.is_empty() {
        if !in_quote {
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

        // A code block can open anywhere on a line.
        if let Some((start, block, len)) = find_code_block(rest, line.len()) {
            paragraph.push_str(&rest[..start]);
            flush(&mut paragraph, blocks);
            blocks.push(block);
            rest = &rest[start + len..];
            // A newline straight after would draw as an empty line.
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

/// Drops the newline leading into the next block but keeps blank lines before
/// it, as Discord does.
fn flush(paragraph: &mut String, blocks: &mut Vec<Block>) {
    let text = paragraph.strip_suffix('\n').unwrap_or(paragraph);
    if !text.is_empty() {
        blocks.push(Block::Paragraph(inline::parse(text, true)));
    }
    paragraph.clear();
}

fn split_line(text: &str) -> (&str, &str) {
    match text.find('\n') {
        Some(end) => (&text[..end], &text[end + 1..]),
        None => (text, ""),
    }
}

/// A bare `>` isn't a quote.
fn quote_line(text: &str) -> Option<&str> {
    let (line, _) = split_line(text);
    if line.starts_with(">>> ") {
        return None;
    }
    line.strip_prefix("> ")
}

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

/// Switching between bullets and numbers starts a new list. Returns the text
/// after the run.
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

/// Deeper lines nest under the item above. Stops at a shallower line, or one
/// of the other kind at the same depth.
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

/// Returns the start, block and byte length of a code block opening within the
/// first `line_len` bytes. An unclosed fence is just text.
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

/// The word after the fence is the language only when a newline and code
/// follow it. Leading and trailing blank lines are dropped.
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
