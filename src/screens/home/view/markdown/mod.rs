//! Draws parsed markdown ([`discord::Markdown`]): message content, embed text,
//! and the reply preview line.
//!
//! Each run of inline text becomes one [`RichText`]; blocks around it
//! (headings, lists, quotes, code) are ordinary elements. Mentions are named
//! at draw time from what the screen has loaded, so a role mention picks up
//! its name as soon as the gateway delivers the guild's roles.

mod inline;
mod rich_text;

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, Block, Inline};
use crate::screens::home::selection::{TextCopy, TextKey, TextOwner, TextPart};
use crate::screens::home::{HomeScreen, View};
use crate::ui::{dialogs, tooltip};

use inline::{Action, Flattener, ResolvedUser};
use rich_text::{RichText, Segment, Selectable};

const EDITED_MARKER: &str = "(edited)";
const EDITED_MARKER_SIZE: f32 = 10.;
const JUMBO_SIZE: f32 = 48.;
const LIST_MARKER_WIDTH: f32 = 20.;
const BLOCK_GAP: f32 = 4.;

pub(super) struct MarkdownOptions<'a> {
    /// Seeds element ids and spoiler keys; must be unique across the message list.
    pub scope: SharedString,
    pub mentions: &'a [discord::MentionedUser],
    pub edited: Option<SharedString>,
    /// Message content only; Discord never enlarges emoji in embeds.
    pub jumbo: bool,
    /// Off where the text sits inside something clickable, like a linked embed
    /// title.
    pub interactive: bool,
    pub selectable: Option<TextOwner>,
}

impl<'a> MarkdownOptions<'a> {
    pub fn new(scope: impl Into<SharedString>, mentions: &'a [discord::MentionedUser]) -> Self {
        Self {
            scope: scope.into(),
            mentions,
            edited: None,
            jumbo: false,
            interactive: true,
            selectable: None,
        }
    }

    pub fn edited(mut self, edited: Option<impl Into<SharedString>>) -> Self {
        self.edited = edited.map(Into::into);
        self
    }

    pub fn jumbo(mut self) -> Self {
        self.jumbo = true;
        self
    }

    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    pub fn selectable(mut self, message: u64, part: TextPart) -> Self {
        self.selectable = Some(TextOwner::new(message, part));
        self
    }
}

struct Palette {
    link: Hsla,
    mention: Hsla,
    mention_background: Hsla,
    code_background: Hsla,
    selection: Hsla,
    /// Opaque: the text under it is the bar's colour, so see-through shows glyphs.
    spoiler_hidden: Hsla,
    spoiler_revealed: Hsla,
    timestamp_background: Hsla,
    muted: Hsla,
    border: Hsla,
}

impl Palette {
    fn new(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            link: theme.link,
            mention: theme.link,
            mention_background: theme.link.opacity(0.15),
            code_background: theme.muted,
            selection: theme.selection,
            spoiler_hidden: theme.background.blend(theme.muted_foreground.opacity(0.6)),
            spoiler_revealed: theme.foreground.opacity(0.08),
            timestamp_background: theme.foreground.opacity(0.08),
            muted: theme.muted_foreground,
            border: theme.border,
        }
    }
}

type ActionHandler = Rc<dyn Fn(&Action, &mut Window, &mut App)>;

struct Renderer<'a> {
    screen: &'a HomeScreen,
    options: MarkdownOptions<'a>,
    palette: Palette,
    mono_family: SharedString,
    on_action: ActionHandler,
    /// Unix seconds.
    now: i64,
    next_text: Cell<usize>,
    next_spoiler: Cell<usize>,
}

impl<'a> Renderer<'a> {
    fn new(screen: &'a HomeScreen, options: MarkdownOptions<'a>, cx: &Context<HomeScreen>) -> Self {
        let weak = cx.entity().downgrade();
        let on_action =
            Rc::new(
                move |action: &Action, window: &mut Window, cx: &mut App| match action {
                    // A drag that selected the link's text wasn't a click on it.
                    _ if weak
                        .read_with(cx, |this, _| this.has_text_selection())
                        .unwrap_or(false) => {}
                    Action::Url(url) => {
                        let opened = discord::MessageLink::parse(url).is_some_and(|link| {
                            weak.update(cx, |this, cx| this.open_message_link(link, window, cx))
                                .unwrap_or(false)
                        });
                        if !opened {
                            cx.open_url(url);
                        }
                    }
                    Action::MaskedUrl(url) => dialogs::confirm_open_link(url.clone(), window, cx),
                    Action::User {
                        id,
                        name,
                        avatar_url,
                    } => {
                        let position = window.mouse_position();
                        let _ = weak.update(cx, |this, cx| {
                            this.open_profile(*id, name.clone(), avatar_url.clone(), position, cx)
                        });
                    }
                    Action::Channel(id) => {
                        let _ = weak.update(cx, |this, cx| {
                            let is_text = this
                                .channel_info(*id)
                                .is_some_and(|channel| !channel.kind.is_voice());
                            if this.view == View::Guild && is_text {
                                this.select_channel(*id, window, cx);
                                cx.notify();
                            }
                        });
                    }
                    Action::Spoiler(key) => {
                        let _ = weak.update(cx, |this, cx| {
                            this.revealed_spoilers.insert(key.clone());
                            cx.notify();
                        });
                    }
                },
            );

        if let Some(owner) = options.selectable {
            screen.text_copies.borrow_mut().clear_part(owner);
        }
        Self {
            screen,
            options,
            palette: Palette::new(cx),
            mono_family: cx.theme().mono_font_family.clone(),
            on_action,
            now: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64),
            next_text: Cell::new(0),
            next_spoiler: Cell::new(0),
        }
    }

    fn next_text(&self) -> (ElementId, Option<TextKey>) {
        let index = self.next_text.replace(self.next_text.get() + 1);
        (
            ElementId::NamedInteger(self.options.scope.clone(), index as u64),
            self.options.selectable.map(|owner| owner.key(index)),
        )
    }

    /// Each custom emoji's blank copies as the name in `emoji`.
    fn selectable(
        &self,
        rich: RichText,
        key: Option<TextKey>,
        text: SharedString,
        emoji: Vec<(Range<usize>, SharedString)>,
    ) -> RichText {
        let Some(key) = key else {
            return rich;
        };
        let screen = self.screen;
        let selected = screen
            .text_selection
            .as_ref()
            .and_then(|selection| selection.range_in(key, text.len()));
        screen
            .text_copies
            .borrow_mut()
            .insert(key, TextCopy::new(text, emoji));
        rich.selectable(Selectable {
            key,
            regions: screen.text_regions.clone(),
            selected,
            color: self.palette.selection,
        })
    }

    fn code(&self, code: &str) -> RichText {
        // gpui draws a tab as nothing at all.
        let text: SharedString = code.replace('\t', "    ").into();
        let segments = if text.is_empty() {
            Vec::new()
        } else {
            vec![Segment {
                len: text.len(),
                style: HighlightStyle::default(),
                mono: true,
            }]
        };
        let (id, key) = self.next_text();
        let rich = RichText::new(
            id,
            text.clone(),
            segments,
            self.mono_family.clone(),
            &self.screen.image_cache,
        );
        self.selectable(rich, key, text, Vec::new())
    }

    /// Stable across frames because the text is always walked in the same order.
    fn next_spoiler(&self) -> SharedString {
        let index = self.next_spoiler.replace(self.next_spoiler.get() + 1);
        format!("{}-spoiler-{index}", self.options.scope).into()
    }

    fn user(&self, id: Id<UserMarker>) -> Option<ResolvedUser> {
        if let Some(user) = self.options.mentions.iter().find(|user| user.id == id) {
            return Some(user.into());
        }
        let screen = self.screen;
        if let Some(user) = screen.current_user.as_ref().filter(|user| user.id == id) {
            return Some(ResolvedUser {
                name: user.name.clone(),
                avatar_url: user.avatar_url.clone(),
            });
        }
        screen.profile_cache.get(&id).map(|profile| ResolvedUser {
            name: profile.name.clone(),
            avatar_url: profile.avatar_url.clone(),
        })
    }

    fn text(&self, inlines: &[Inline], edited: bool) -> impl IntoElement {
        let mut flattener = Flattener::new(self);
        flattener.inlines(inlines);
        if edited && let Some(when) = &self.options.edited {
            flattener.marker(
                EDITED_MARKER,
                px(EDITED_MARKER_SIZE),
                self.palette.muted,
                when.clone(),
            );
        }
        div().w_full().min_w_0().child(flattener.finish())
    }

    fn blocks(&self, blocks: &[Block], edited: bool) -> Div {
        let last = blocks.len().saturating_sub(1);
        // After a list, quote or code block it gets a line of its own.
        let inline_marker = edited && matches!(blocks.last(), Some(Block::Paragraph(_)));
        v_flex()
            .w_full()
            .min_w_0()
            .gap(px(BLOCK_GAP))
            .children(
                blocks
                    .iter()
                    .enumerate()
                    .map(|(ix, block)| self.block(block, inline_marker && ix == last)),
            )
            .when(edited && !inline_marker, |this| {
                this.children(self.edited_marker())
            })
    }

    fn edited_marker(&self) -> Option<impl IntoElement> {
        let when = self.options.edited.clone()?;
        Some(
            div()
                .id(ElementId::Name(
                    format!("{}-edited", self.options.scope).into(),
                ))
                .text_size(px(EDITED_MARKER_SIZE))
                .text_color(self.palette.muted)
                .child(EDITED_MARKER)
                .when(self.options.interactive, |this| {
                    this.tooltip(tooltip::text(when))
                }),
        )
    }

    fn block(&self, block: &Block, edited: bool) -> AnyElement {
        match block {
            Block::Paragraph(inlines) => self.text(inlines, edited).into_any_element(),
            Block::Heading { level, content } => {
                let (size, line_height) = match level {
                    1 => (22., 28.),
                    2 => (18., 24.),
                    _ => (16., 22.),
                };
                div()
                    .w_full()
                    .min_w_0()
                    .text_size(px(size))
                    .line_height(px(line_height))
                    .font_weight(FontWeight::BOLD)
                    .child(self.text(content, edited))
                    .into_any_element()
            }
            Block::Subtext(inlines) => div()
                .w_full()
                .min_w_0()
                .text_xs()
                .line_height(px(16.))
                .text_color(self.palette.muted)
                .child(self.text(inlines, edited))
                .into_any_element(),
            Block::List(list) => self.list(list, 0).into_any_element(),
            Block::Quote(blocks) => div()
                .w_full()
                .min_w_0()
                .border_l(px(4.))
                .border_color(self.palette.border)
                .pl(px(12.))
                .child(self.blocks(blocks, false))
                .into_any_element(),
            Block::Code { code, .. } => div()
                .w_full()
                .min_w_0()
                .p(px(8.))
                .rounded(px(4.))
                .bg(self.palette.code_background)
                .border_1()
                .border_color(self.palette.border)
                .font_family(self.mono_family.clone())
                .text_size(px(12.5))
                .line_height(px(18.))
                .child(self.code(code))
                .into_any_element(),
        }
    }

    fn list(&self, list: &discord::List, depth: usize) -> impl IntoElement {
        let bullet = match depth {
            0 => "•",
            1 => "◦",
            _ => "▪",
        };
        v_flex()
            .w_full()
            .min_w_0()
            .gap(px(2.))
            .children(list.items.iter().enumerate().map(|(ix, item)| {
                let marker = match list.start {
                    Some(start) => format!("{}.", start + ix as u64),
                    None => bullet.to_string(),
                };
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .child(
                        div()
                            .flex_shrink_0()
                            .min_w(px(LIST_MARKER_WIDTH))
                            .pr(px(4.))
                            .child(marker),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(self.text(&item.content, false))
                            .children(
                                item.children
                                    .iter()
                                    .map(|child| self.list(child, depth + 1)),
                            ),
                    )
            }))
    }

    fn jumbo(&self, inlines: &[Inline], edited: bool) -> impl IntoElement {
        let scope = &self.options.scope;
        let mut items: Vec<AnyElement> = Vec::new();
        for (ix, inline) in inlines.iter().enumerate() {
            match inline {
                Inline::Emoji(emoji) => items.push(
                    div()
                        .id(ElementId::NamedInteger(
                            format!("{scope}-jumbo").into(),
                            ix as u64,
                        ))
                        .flex_shrink_0()
                        .child(
                            img(emoji.url())
                                .image_cache(&self.screen.image_cache)
                                .size(px(JUMBO_SIZE)),
                        )
                        .when(self.options.interactive, |this| {
                            this.tooltip(tooltip::text(format!(":{}:", emoji.name)))
                        })
                        .into_any_element(),
                ),
                Inline::Text(text) => items.extend(text.split_whitespace().map(|word| {
                    div()
                        .flex_shrink_0()
                        .text_size(px(JUMBO_SIZE * 0.8))
                        .line_height(px(JUMBO_SIZE))
                        .child(word.to_string())
                        .into_any_element()
                })),
                _ => {}
            }
        }
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_end()
            .gap(px(2.))
            .children(items)
            .when(edited, |this| this.children(self.edited_marker()))
    }
}

impl HomeScreen {
    pub(super) fn render_markdown(
        &self,
        markdown: &discord::Markdown,
        options: MarkdownOptions<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let jumbo = options.jumbo && markdown.jumbo;
        let edited = options.edited.is_some();
        let renderer = Renderer::new(self, options, cx);
        if jumbo && let [Block::Paragraph(inlines)] = markdown.blocks.as_slice() {
            return renderer.jumbo(inlines, edited).into_any_element();
        }
        renderer.blocks(&markdown.blocks, edited).into_any_element()
    }

    pub(super) fn render_inline_markdown(
        &self,
        inlines: &[Inline],
        options: MarkdownOptions<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let renderer = Renderer::new(self, options, cx);
        let mut flattener = Flattener::new(&renderer);
        flattener.inlines(inlines);
        flattener.finish().into_any_element()
    }
}
