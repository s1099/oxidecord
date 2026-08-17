//! Rich rendering of message text: for now, clickable links.

use std::ops::Range;
use std::sync::LazyLock;

use gpui::*;
use regex::Regex;

/// Runs each URL up to the next whitespace or angle bracket; trailing prose
/// punctuation is trimmed separately, in [`find_links`].
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>]+").expect("valid url regex"));

struct Link {
    range: Range<usize>,
    url: String,
}

/// Finds every `http`/`https` URL in `text`, in order of appearance. Trailing
/// punctuation that usually belongs to the prose rather than the link (a
/// sentence's period, a wrapping paren, ...) is left out of the match.
fn find_links(text: &str) -> Vec<Link> {
    URL_REGEX
        .find_iter(text)
        .map(|m| {
            let url = m.as_str().trim_end_matches(|c| {
                matches!(
                    c,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
                )
            });
            Link {
                range: m.start()..m.start() + url.len(),
                url: url.to_string(),
            }
        })
        .collect()
}

/// Renders message text with any `http`/`https` URLs shown in the theme's link
/// colour, underlined, and clickable — a click opens the URL in the default
/// browser. Text without links renders as a plain string.
pub(in crate::screens::home::view) fn render_message_text(
    id: u64,
    content: &str,
    link_color: Hsla,
) -> AnyElement {
    let links = find_links(content);
    if links.is_empty() {
        return div()
            .w_full()
            .min_w_0()
            .child(content.to_string())
            .into_any_element();
    }

    let highlight = HighlightStyle {
        color: Some(link_color),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(link_color),
            wavy: false,
        }),
        ..Default::default()
    };
    let ranges: Vec<Range<usize>> = links.iter().map(|link| link.range.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|link| link.url).collect();
    let highlights: Vec<(Range<usize>, HighlightStyle)> = ranges
        .iter()
        .map(|range| (range.clone(), highlight))
        .collect();

    // `with_highlights` computes the plain runs from the ambient text style at
    // layout time, so the non-link text keeps the surrounding size and colour;
    // only the link ranges get the highlight overlaid.
    let styled = StyledText::new(content.to_string()).with_highlights(highlights);
    div()
        .w_full()
        .min_w_0()
        .child(
            InteractiveText::new(("message-content", id), styled).on_click(
                ranges,
                move |ix, _window, cx| {
                    if let Some(url) = urls.get(ix) {
                        cx.open_url(url);
                    }
                },
            ),
        )
        .into_any_element()
}
