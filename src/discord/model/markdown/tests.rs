use twilight_model::id::Id;

use super::*;

fn text(s: &str) -> Inline {
    Inline::Text(s.to_string())
}

fn inline(s: &str) -> Vec<Inline> {
    inline::parse(s, true)
}

fn paragraph(s: &str) -> Vec<Inline> {
    match Markdown::parse(s).blocks.as_slice() {
        [Block::Paragraph(inlines)] => inlines.clone(),
        other => panic!("expected one paragraph, got {other:?}"),
    }
}

#[test]
fn emphasis() {
    assert_eq!(inline("**bold**"), [Inline::Bold(vec![text("bold")])]);
    assert_eq!(inline("*it*"), [Inline::Italic(vec![text("it")])]);
    assert_eq!(inline("_it_"), [Inline::Italic(vec![text("it")])]);
    assert_eq!(
        inline("__under__"),
        [Inline::Underline(vec![text("under")])]
    );
    assert_eq!(
        inline("~~gone~~"),
        [Inline::Strikethrough(vec![text("gone")])]
    );
    assert_eq!(
        inline("||secret||"),
        [Inline::Spoiler(vec![text("secret")])]
    );
    assert_eq!(
        inline("***both***"),
        [Inline::Italic(vec![Inline::Bold(vec![text("both")])])]
    );
    assert_eq!(
        inline("__*mixed*__"),
        [Inline::Underline(vec![Inline::Italic(vec![text("mixed")])])]
    );
}

#[test]
fn emphasis_edge_cases() {
    // Snake case isn't italic.
    assert_eq!(inline("snake_case_name"), [text("snake_case_name")]);
    // An italic can't open or close on a space.
    assert_eq!(inline("* not *"), [text("* not *")]);
    assert_eq!(inline("2 * 3 * 4"), [text("2 * 3 * 4")]);
    // Unclosed delimiters are text.
    assert_eq!(inline("**open"), [text("**open")]);
    assert_eq!(inline("a || b"), [text("a || b")]);
}

#[test]
fn escapes() {
    assert_eq!(inline(r"\*not italic\*"), [text("*not italic*")]);
    assert_eq!(inline(r"\<@123>"), [text("<@123>")]);
    assert_eq!(inline(r"a\b"), [text(r"a\b")]);
}

#[test]
fn code_spans() {
    assert_eq!(inline("`**raw**`"), [Inline::Code("**raw**".into())]);
    assert_eq!(inline("``a ` b``"), [Inline::Code("a ` b".into())]);
    assert_eq!(inline("`open"), [text("`open")]);
}

#[test]
fn links() {
    assert_eq!(
        inline("see https://example.com."),
        [
            text("see "),
            Inline::Link {
                url: "https://example.com".into(),
                label: None
            },
            text(".")
        ]
    );
    assert_eq!(
        inline("(https://en.wikipedia.org/wiki/Rust_(language))"),
        [
            text("("),
            Inline::Link {
                url: "https://en.wikipedia.org/wiki/Rust_(language)".into(),
                label: None
            },
            text(")")
        ]
    );
    assert_eq!(
        inline("<https://quiet.example>"),
        [Inline::Link {
            url: "https://quiet.example".into(),
            label: None
        }]
    );
    assert_eq!(
        inline("[**docs**](https://docs.rs \"title\")!"),
        [
            Inline::Link {
                url: "https://docs.rs".into(),
                label: Some(vec![Inline::Bold(vec![text("docs")])])
            },
            text("!")
        ]
    );
    assert_eq!(
        inline("[x](javascript:alert(1))"),
        [text("[x](javascript:alert(1))")]
    );
    // No links inside a masked link's label.
    assert_eq!(
        inline("[https://a.example](https://b.example)"),
        [Inline::Link {
            url: "https://b.example".into(),
            label: Some(vec![text("https://a.example")])
        }]
    );
}

#[test]
fn tokens() {
    assert_eq!(
        inline("<@1> <@!2> <@&3> <#4> @everyone @here"),
        [
            Inline::Mention(Mention::User(Id::new(1))),
            text(" "),
            Inline::Mention(Mention::User(Id::new(2))),
            text(" "),
            Inline::Mention(Mention::Role(Id::new(3))),
            text(" "),
            Inline::Mention(Mention::Channel(Id::new(4))),
            text(" "),
            Inline::Mention(Mention::Everyone),
            text(" "),
            Inline::Mention(Mention::Here),
        ]
    );
    assert_eq!(
        inline("<a:party:55>"),
        [Inline::Emoji(CustomEmoji {
            id: Id::new(55),
            name: "party".into(),
            animated: true
        })]
    );
    assert_eq!(
        inline("<t:0:R></ping:9><id:browse>"),
        [
            Inline::Timestamp(Timestamp {
                unix: 0,
                style: TimestampStyle::Relative
            }),
            Inline::Mention(Mention::Command("ping".into())),
            Inline::Mention(Mention::GuildNavigation(GuildNavigation::Browse)),
        ]
    );
    // A zero id isn't a snowflake.
    assert_eq!(inline("<@0>"), [text("<@0>")]);
}

#[test]
fn blocks() {
    let parsed = Markdown::parse("# Title\nbody\n-# small\n## Two");
    assert_eq!(
        parsed.blocks,
        [
            Block::Heading {
                level: 1,
                content: vec![text("Title")]
            },
            Block::Paragraph(vec![text("body")]),
            Block::Subtext(vec![text("small")]),
            Block::Heading {
                level: 2,
                content: vec![text("Two")]
            },
        ]
    );
    // Not headings: no space, or too deep.
    assert_eq!(paragraph("#tag"), [text("#tag")]);
    assert_eq!(paragraph("#### four"), [text("#### four")]);
    // Line breaks and blank lines are kept.
    assert_eq!(paragraph("a\n\nb"), [text("a\n\nb")]);
}

#[test]
fn quotes() {
    assert_eq!(
        Markdown::parse("> one\n> **two**\nafter").blocks,
        [
            Block::Quote(vec![Block::Paragraph(vec![
                text("one\n"),
                Inline::Bold(vec![text("two")])
            ])]),
            Block::Paragraph(vec![text("after")]),
        ]
    );
    assert_eq!(
        Markdown::parse("before\n>>> all\n> of\nthis").blocks,
        [
            Block::Paragraph(vec![text("before")]),
            Block::Quote(vec![Block::Paragraph(vec![text("all\n> of\nthis")])]),
        ]
    );
    assert_eq!(paragraph(">no space"), [text(">no space")]);
}

#[test]
fn lists() {
    let parsed = Markdown::parse("- a\n  - nested\n- b\n3. three\n4. four");
    assert_eq!(
        parsed.blocks,
        [
            Block::List(List {
                start: None,
                items: vec![
                    ListItem {
                        content: vec![text("a")],
                        children: vec![List {
                            start: None,
                            items: vec![ListItem {
                                content: vec![text("nested")],
                                children: vec![]
                            }]
                        }]
                    },
                    ListItem {
                        content: vec![text("b")],
                        children: vec![]
                    },
                ]
            }),
            Block::List(List {
                start: Some(3),
                items: vec![
                    ListItem {
                        content: vec![text("three")],
                        children: vec![]
                    },
                    ListItem {
                        content: vec![text("four")],
                        children: vec![]
                    },
                ]
            }),
        ]
    );
    // An italic at the start of a line isn't a bullet.
    assert_eq!(
        paragraph("*not a list*"),
        [Inline::Italic(vec![text("not a list")])]
    );
}

#[test]
fn code_blocks() {
    assert_eq!(
        Markdown::parse("look:\n```rs\nfn main() {}\n```\ndone").blocks,
        [
            Block::Paragraph(vec![text("look:")]),
            Block::Code {
                language: Some("rs".into()),
                code: "fn main() {}".into()
            },
            Block::Paragraph(vec![text("done")]),
        ]
    );
    assert_eq!(
        Markdown::parse("```one line```").blocks,
        [Block::Code {
            language: None,
            code: "one line".into()
        }]
    );
    // The word is only a language when code follows it.
    assert_eq!(
        Markdown::parse("```py\n```").blocks,
        [Block::Code {
            language: None,
            code: "py".into()
        }]
    );
    // Markup inside is left alone.
    assert_eq!(
        Markdown::parse("```\n# not a heading\n**raw**\n```").blocks,
        [Block::Code {
            language: None,
            code: "# not a heading\n**raw**".into()
        }]
    );
}

#[test]
fn jumbo() {
    assert!(Markdown::parse("😀 👍🏽 <:wave:1>").jumbo);
    assert!(Markdown::parse("👨‍👩‍👧 🇺🇸 1️⃣").jumbo);
    assert!(!Markdown::parse("hi 😀").jumbo);
    assert!(!Markdown::parse("1").jumbo);
    assert!(!Markdown::parse(&"😀".repeat(31)).jumbo);
}

#[test]
fn timestamps() {
    let stamp = Timestamp {
        unix: 1_790_000_000,
        style: TimestampStyle::LongDateTime,
    };
    assert_eq!(stamp.display(0), "Monday, September 21, 2026 2:13 PM");
    let relative = Timestamp {
        unix: 1_000,
        style: TimestampStyle::Relative,
    };
    assert_eq!(relative.display(1_000 + 7_200), "2 hours ago");
    assert_eq!(relative.display(1_000 - 60), "in 1 minute");
}
