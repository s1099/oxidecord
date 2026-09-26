//! Flattens a tree of inline nodes into one string with styled segments, the
//! shape [`RichText`] draws. Mentions are named here, against whatever the
//! screen has loaded by now.

use std::ops::Range;

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, UserMarker},
};

use crate::discord::{self, Inline, Mention};

use super::Renderer;
use super::rich_text::{EMOJI_PLACEHOLDER, RichText, Segment};

/// What clicking a range does.
#[derive(Clone)]
pub(super) enum Action {
    /// A bare URL, opened straight away: what it says is where it goes.
    Url(String),
    /// A masked link, whose text can claim anything, so it asks first.
    MaskedUrl(String),
    User {
        id: Id<UserMarker>,
        name: String,
        avatar_url: Option<String>,
    },
    Channel(Id<ChannelMarker>),
    /// Reveals the spoiler with this key.
    Spoiler(SharedString),
}

/// The formatting in effect at a point in the tree.
#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    mono: bool,
    color: Option<Hsla>,
    background: Option<Hsla>,
    /// Inside an unrevealed spoiler: drawn as a solid bar, with nothing in it
    /// clickable, hoverable or painted over it.
    hidden: bool,
    /// Inside a link, whose click wins over a mention's.
    linked: bool,
}

impl Style {
    /// Colours the text, unless it's hidden in a spoiler, which keeps its
    /// solid bar whatever sits inside.
    fn tinted(self, color: Option<Hsla>, background: Option<Hsla>) -> Self {
        if self.hidden {
            return self;
        }
        Self {
            color: color.or(self.color),
            background: background.or(self.background),
            ..self
        }
    }

    fn highlight(self) -> HighlightStyle {
        HighlightStyle {
            color: self.color,
            background_color: self.background,
            font_weight: self.bold.then_some(FontWeight::BOLD),
            font_style: self.italic.then_some(FontStyle::Italic),
            underline: self.underline.then_some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            }),
            strikethrough: self.strikethrough.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            fade_out: None,
        }
    }
}

/// Builds one [`RichText`] from inline nodes.
pub(super) struct Flattener<'a, 'b> {
    renderer: &'a Renderer<'b>,
    text: String,
    segments: Vec<Segment>,
    actions: Vec<(Range<usize>, Action)>,
    tooltips: Vec<(Range<usize>, SharedString)>,
    emoji: Vec<(Range<usize>, SharedString)>,
}

impl<'a, 'b> Flattener<'a, 'b> {
    pub fn new(renderer: &'a Renderer<'b>) -> Self {
        Self {
            renderer,
            text: String::new(),
            segments: Vec::new(),
            actions: Vec::new(),
            tooltips: Vec::new(),
            emoji: Vec::new(),
        }
    }

    pub fn inlines(&mut self, nodes: &[Inline]) -> &mut Self {
        self.nodes(nodes, Style::default());
        self
    }

    /// Appends text outside the markup, such as the "(edited)" marker.
    pub fn trailing(&mut self, text: &str, color: Hsla) -> &mut Self {
        self.push(
            text,
            Style {
                color: Some(color),
                ..Style::default()
            },
        );
        self
    }

    pub fn finish(self) -> RichText {
        let renderer = self.renderer;
        let mut text = RichText::new(
            renderer.next_id(),
            self.text,
            self.segments,
            renderer.mono_family.clone(),
            &renderer.screen.image_cache,
        )
        .emoji(self.emoji);
        if renderer.options.interactive {
            let (ranges, actions): (Vec<_>, Vec<_>) = self.actions.into_iter().unzip();
            let on_action = renderer.on_action.clone();
            text = text
                .on_click(ranges, move |ix, window, cx| {
                    if let Some(action) = actions.get(ix) {
                        on_action(action, window, cx);
                    }
                })
                .tooltips(self.tooltips);
        }
        text
    }

    fn push(&mut self, text: &str, style: Style) -> Range<usize> {
        let start = self.text.len();
        self.text.push_str(text);
        if !text.is_empty() {
            self.segments.push(Segment {
                len: text.len(),
                style: style.highlight(),
                mono: style.mono,
            });
        }
        start..self.text.len()
    }

    fn nodes(&mut self, nodes: &[Inline], style: Style) {
        for node in nodes {
            self.node(node, style);
        }
    }

    fn node(&mut self, node: &Inline, style: Style) {
        let palette = &self.renderer.palette;
        match node {
            Inline::Text(text) => {
                self.push(text, style);
            }
            Inline::Bold(children) => self.nodes(
                children,
                Style {
                    bold: true,
                    ..style
                },
            ),
            Inline::Italic(children) => self.nodes(
                children,
                Style {
                    italic: true,
                    ..style
                },
            ),
            Inline::Underline(children) => self.nodes(
                children,
                Style {
                    underline: true,
                    ..style
                },
            ),
            Inline::Strikethrough(children) => self.nodes(
                children,
                Style {
                    strikethrough: true,
                    ..style
                },
            ),
            Inline::Spoiler(children) => {
                let key = self.renderer.next_spoiler();
                if style.hidden || self.renderer.screen.revealed_spoilers.contains(&key) {
                    let revealed = style.tinted(None, Some(palette.spoiler_revealed));
                    self.nodes(children, revealed);
                } else {
                    let hidden = Style {
                        color: Some(palette.spoiler_hidden),
                        background: Some(palette.spoiler_hidden),
                        hidden: true,
                        ..style
                    };
                    let start = self.text.len();
                    self.nodes(children, hidden);
                    self.actions
                        .push((start..self.text.len(), Action::Spoiler(key)));
                }
            }
            Inline::Code(code) => {
                let style = Style {
                    mono: true,
                    ..style.tinted(None, Some(palette.code_background))
                };
                self.push(code, style);
            }
            Inline::Link { url, label } => {
                let linked = Style {
                    underline: !style.hidden,
                    linked: true,
                    ..style.tinted(Some(palette.link), None)
                };
                let start = self.text.len();
                match label {
                    Some(label) => self.nodes(label, linked),
                    None => {
                        self.push(url, linked);
                    }
                }
                let range = start..self.text.len();
                if !style.hidden {
                    let action = match label {
                        Some(_) => {
                            self.tooltips.push((range.clone(), url.clone().into()));
                            Action::MaskedUrl(url.clone())
                        }
                        None => Action::Url(url.clone()),
                    };
                    self.actions.push((range, action));
                }
            }
            Inline::Mention(mention) => self.mention(mention, style),
            Inline::Emoji(emoji) => {
                let range = self.push(EMOJI_PLACEHOLDER, style);
                if !style.hidden {
                    self.emoji.push((range.clone(), emoji.url().into()));
                    self.tooltips
                        .push((range, format!(":{}:", emoji.name).into()));
                }
            }
            Inline::Timestamp(timestamp) => {
                let shown = timestamp.display(self.renderer.now);
                let range = self.push(
                    &shown,
                    style.tinted(None, Some(palette.timestamp_background)),
                );
                if !style.hidden {
                    self.tooltips.push((range, timestamp.full().into()));
                }
            }
        }
    }

    fn mention(&mut self, mention: &Mention, style: Style) {
        let palette = &self.renderer.palette;
        let screen = self.renderer.screen;
        let pill = style.tinted(Some(palette.mention), Some(palette.mention_background));

        let (text, style, action) = match mention {
            Mention::User(id) => match self.renderer.user(*id) {
                Some(user) => (
                    format!("@{}", user.name),
                    pill,
                    Some(Action::User {
                        id: *id,
                        name: user.name,
                        avatar_url: user.avatar_url,
                    }),
                ),
                None => ("@unknown-user".to_string(), pill, None),
            },
            Mention::Role(id) => {
                let role = screen
                    .selected_guild
                    .and_then(|guild| screen.guild_roles.get(&guild)?.get(id));
                match role {
                    Some(role) => {
                        // A coloured role takes its colour, the way its
                        // members' names do.
                        let style = match role.color {
                            Some(color) => {
                                let color: Hsla = rgb(color).into();
                                style.tinted(Some(color), Some(color.opacity(0.12)))
                            }
                            None => pill,
                        };
                        (format!("@{}", role.name), style, None)
                    }
                    None => ("@deleted-role".to_string(), pill, None),
                }
            }
            Mention::Channel(id) => match screen.channel_info(*id) {
                Some(channel) => (
                    format!("#{}", channel.name),
                    pill,
                    Some(Action::Channel(*id)),
                ),
                None => ("#unknown".to_string(), pill, None),
            },
            Mention::Everyone => ("@everyone".to_string(), pill, None),
            Mention::Here => ("@here".to_string(), pill, None),
            Mention::Command(name) => (format!("/{name}"), pill, None),
            Mention::GuildNavigation(target) => (target.label().to_string(), pill, None),
        };

        let range = self.push(&text, style);
        if let Some(action) = action.filter(|_| !style.hidden && !style.linked) {
            self.actions.push((range, action));
        }
    }
}

/// Who a user mention names, from the message's own mention list first and
/// then anyone the app already knows.
pub(super) struct ResolvedUser {
    pub name: String,
    pub avatar_url: Option<String>,
}

impl From<&discord::MentionedUser> for ResolvedUser {
    fn from(user: &discord::MentionedUser) -> Self {
        Self {
            name: user.name.clone(),
            avatar_url: user.avatar_url.clone(),
        }
    }
}
