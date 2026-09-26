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
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};
use twilight_model::id::{Id, marker::UserMarker};

use crate::discord::{self, Block, Inline};
use crate::screens::home::{HomeScreen, View};
use crate::ui::{dialogs, tooltip};

use inline::{Action, Flattener, ResolvedUser};

/// Follows the text of a message that's been edited, set in a muted colour.
const EDITED_MARKER: &str = " (edited)";
/// How big emoji are drawn in a message of nothing but emoji.
const JUMBO_SIZE: f32 = 48.;
/// The width a list's markers are set in, so item text lines up down a list.
const LIST_MARKER_WIDTH: f32 = 20.;
/// Vertical space between blocks.
const BLOCK_GAP: f32 = 4.;

/// How a piece of markdown is drawn.
pub(super) struct MarkdownOptions<'a> {
    /// Seeds element ids and spoiler keys, so it has to be unique across the
    /// message list.
    pub scope: SharedString,
    /// The users the text's message mentions, which name its `<@id>`s.
    pub mentions: &'a [discord::MentionedUser],
    /// Ends the text with "(edited)".
    pub edited: bool,
    /// Draws a message of only emoji large. Message content only; Discord
    /// never does it in embeds.
    pub jumbo: bool,
    /// Whether links, mentions and spoilers respond to clicks, and anything
    /// has a tooltip. Off where the text sits inside something clickable of
    /// its own, like a linked embed title.
    pub interactive: bool,
}

impl<'a> MarkdownOptions<'a> {
    pub fn new(scope: impl Into<SharedString>, mentions: &'a [discord::MentionedUser]) -> Self {
        Self {
            scope: scope.into(),
            mentions,
            edited: false,
            jumbo: false,
            interactive: true,
        }
    }

    pub fn edited(mut self, edited: bool) -> Self {
        self.edited = edited;
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
}

/// The theme colours markdown is drawn in.
struct Palette {
    link: Hsla,
    mention: Hsla,
    mention_background: Hsla,
    code_background: Hsla,
    /// Opaque, since the text under a hidden spoiler is set in the same
    /// colour as its bar — anything see-through would show the glyphs.
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
            spoiler_hidden: theme.background.blend(theme.muted_foreground.opacity(0.6)),
            spoiler_revealed: theme.foreground.opacity(0.08),
            timestamp_background: theme.foreground.opacity(0.08),
            muted: theme.muted_foreground,
            border: theme.border,
        }
    }
}

type ActionHandler = Rc<dyn Fn(&Action, &mut Window, &mut App)>;

/// Everything one call to render markdown shares.
struct Renderer<'a> {
    screen: &'a HomeScreen,
    options: MarkdownOptions<'a>,
    palette: Palette,
    mono_family: SharedString,
    on_action: ActionHandler,
    /// Unix seconds, for relative timestamps.
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
                    Action::Url(url) => cx.open_url(url),
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

    /// An element id for the next run of text.
    fn next_id(&self) -> ElementId {
        let index = self.next_text.replace(self.next_text.get() + 1);
        ElementId::NamedInteger(self.options.scope.clone(), index as u64)
    }

    /// The key of the next spoiler, stable across frames since the text is
    /// always walked in the same order.
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
        if edited {
            flattener.trailing(EDITED_MARKER, self.palette.muted);
        }
        div().w_full().min_w_0().child(flattener.finish())
    }

    fn blocks(&self, blocks: &[Block], edited: bool) -> Div {
        let last = blocks.len().saturating_sub(1);
        // "(edited)" joins the last line of text when the text ends in some;
        // after a list, quote or code block it gets a line of its own.
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
                this.child(
                    div()
                        .text_xs()
                        .text_color(self.palette.muted)
                        .child(EDITED_MARKER.trim_start()),
                )
            })
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
                // gpui draws a tab as nothing at all.
                .child(code.replace('\t', "    "))
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

    /// A message of only emoji, drawn large and wrapped like words.
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
            .when(edited, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(self.palette.muted)
                        .child(EDITED_MARKER.trim_start()),
                )
            })
    }
}

impl HomeScreen {
    /// Draws a whole piece of markdown: message content, or an embed's
    /// description or field value.
    pub(super) fn render_markdown(
        &self,
        markdown: &discord::Markdown,
        options: MarkdownOptions<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let jumbo = options.jumbo && markdown.jumbo;
        let edited = options.edited;
        let renderer = Renderer::new(self, options, cx);
        if jumbo && let [Block::Paragraph(inlines)] = markdown.blocks.as_slice() {
            return renderer.jumbo(inlines, edited).into_any_element();
        }
        renderer.blocks(&markdown.blocks, edited).into_any_element()
    }

    /// Draws inline markdown as one run of text: an embed title or field name,
    /// or a reply's preview line.
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
