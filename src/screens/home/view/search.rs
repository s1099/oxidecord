//! Message search: the box in the conversation header with the filter
//! suggestions under it, and the results panel that takes the member list's
//! place.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Icon, IconName, Sizable as _,
    StyledExt as _, h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord::{SearchScope, SearchSort};
use crate::screens::home::autocomplete::Suggestion;
use crate::screens::home::channels::channel_icon_path;
use crate::screens::home::search::SearchSuggestion;
use crate::screens::home::view::avatar;
use crate::screens::home::{
    HomeScreen, SEARCH_CONTEXT, SearchAccept, SearchNext, SearchPrevious, SearchSubmit,
};
use crate::ui::button::Button;
use crate::ui::depth::{self, Level, radius};

use super::message::{CARD_LIST_WIDTH, LIST_GAP};

const BOX_WIDTH: f32 = 200.;
const BOX_HEIGHT: f32 = 28.;
const POPOUT_WIDTH: f32 = 320.;
const ROW_HEIGHT: f32 = 32.;
const PANEL_HEADER_HEIGHT: f32 = 48.;
/// Fits three digits: Discord stops paging at page 400.
const PAGE_BOX_WIDTH: f32 = 52.;

impl HomeScreen {
    pub(in crate::screens::home) fn render_search_box(
        &self,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let has_text = !self.search_input.read(cx).value().is_empty();

        // A clear button once there's something to clear, as Discord's box has.
        let suffix = if has_text || self.search.is_some() {
            // A bare icon, not a button: a button's padding makes the input taller.
            div()
                .id("clear-search")
                .flex()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|style| style.text_color(theme.foreground))
                .child(Icon::new(IconName::Close).small())
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx)))
                .into_any_element()
        } else {
            Icon::new(IconName::Search)
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element()
        };

        div()
            .relative()
            .w(px(BOX_WIDTH))
            .flex_shrink_0()
            .key_context(SEARCH_CONTEXT)
            .on_action(cx.listener(|this, _: &SearchSubmit, window, cx| {
                this.on_search_submit(&SearchSubmit, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SearchPrevious, window, cx| {
                this.on_search_previous(&SearchPrevious, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SearchNext, window, cx| {
                this.on_search_next(&SearchNext, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SearchAccept, window, cx| {
                this.on_search_accept(&SearchAccept, window, cx)
            }))
            .on_action(cx.listener(Self::on_search_escape))
            .child(
                // Taller than the header's buttons, as Discord's box is; still well inside the
                // header. The default padding would overflow the trimmed height.
                Input::new(&self.search_input)
                    .h(px(BOX_HEIGHT))
                    .py(px(4.))
                    .focus_ring(false)
                    .suffix(suffix),
            )
            .children(self.render_search_suggestions(cx))
    }

    fn render_search_suggestions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let suggestions = self.search_suggestions.as_ref()?;
        let theme = cx.theme();

        let title = match suggestions.kind {
            None => "Search Options".to_string(),
            Some(kind) => format!("{}:", kind.key()),
        };
        let rows = suggestions
            .items
            .iter()
            .enumerate()
            .map(|(ix, suggestion)| {
                self.render_search_suggestion(suggestion, ix, suggestions.selected == Some(ix), cx)
            });

        let card = v_flex()
            .occlude()
            .w(px(POPOUT_WIDTH))
            .p_2()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(depth::ring(cx))
            .rounded(radius::CARD)
            .shadow(depth::shadow(Level::Overlay, cx))
            // Else a click takes focus from the box, which closes the list.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb(px(6.))
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(title),
            )
            .children(rows);

        Some(
            div()
                .absolute()
                .top_full()
                .right_0()
                .mt_1()
                .child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(card),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }

    fn render_search_suggestion(
        &self,
        suggestion: &SearchSuggestion,
        ix: usize,
        selected: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (glyph, label, detail): (Option<AnyElement>, SharedString, Option<SharedString>) =
            match suggestion {
                SearchSuggestion::Key(kind) => (
                    None,
                    format!("{}:", kind.key()).into(),
                    Some(kind.hint().into()),
                ),
                SearchSuggestion::Name(
                    _,
                    Suggestion::User {
                        name,
                        username,
                        avatar_url,
                        ..
                    },
                ) => (
                    Some(avatar(name.clone(), avatar_url.clone(), px(20.)).into_any_element()),
                    name.clone().into(),
                    username.clone().map(Into::into),
                ),
                SearchSuggestion::Name(
                    _,
                    Suggestion::Channel {
                        name,
                        kind,
                        category,
                        ..
                    },
                ) => (
                    Some(
                        Icon::default()
                            .path(channel_icon_path(*kind))
                            .small()
                            .text_color(theme.muted_foreground)
                            .into_any_element(),
                    ),
                    name.clone().into(),
                    category.clone().map(Into::into),
                ),
                SearchSuggestion::Name(..) => return div().into_any_element(),
                SearchSuggestion::Value { value, detail, .. } => {
                    (None, value.clone().into(), detail.map(Into::into))
                }
            };

        h_flex()
            .id(("search-suggestion", ix))
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.accent))
            .children(glyph.map(|glyph| {
                div()
                    .flex()
                    .size(px(20.))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(glyph)
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(relative(0.6))
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .when(selected, |this| this.text_color(theme.accent_foreground))
                    .child(label),
            )
            .children(detail.map(|detail| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(detail)
            }))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hover_search_suggestion(ix, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.accept_search_suggestion(ix, window, cx);
            }))
            .into_any_element()
    }

    /// In the member list's place while there are results to show.
    pub(in crate::screens::home) fn render_search_panel(
        &self,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let panel = self
            .search
            .as_ref()
            .filter(|panel| Some(panel.scope) == self.search_scope())?;
        let theme = cx.theme();

        let status = |text: SharedString, spinner: bool| {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .px_6()
                .text_sm()
                .text_center()
                .text_color(theme.muted_foreground)
                .when(spinner, |this| {
                    this.child(Spinner::new().color(theme.muted_foreground))
                })
                .child(text)
                .into_any_element()
        };
        let body = if let Some(error) = &panel.error {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_6()
                .text_sm()
                .text_center()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element()
        } else if panel.indexing {
            status(
                "Discord is still indexing these messages. Results will show up in a moment."
                    .into(),
                true,
            )
        } else if panel.loading {
            status("Searching…".into(), true)
        } else if panel.hits.is_empty() {
            status(
                "No results. Try fewer filters or different words.".into(),
                false,
            )
        } else {
            let entity = cx.entity();
            list(panel.list.clone(), move |ix, _window, cx| {
                entity.update(cx, |this, cx| this.render_search_hit(ix, cx))
            })
            .flex_1()
            .into_any_element()
        };

        let title = if panel.loading || panel.error.is_some() {
            "Search".to_string()
        } else {
            match panel.total {
                1 => "1 Result".to_string(),
                total => format!("{total} Results"),
            }
        };
        let sort = panel.filters.sort;
        let screen = cx.entity().downgrade();
        let sort_menu = Button::new("search-sort")
            .icon(Icon::default().path("icons/arrow-down-up.svg"))
            .label("Sort")
            .outline()
            .small()
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                [
                    ("Newest", SearchSort::Newest),
                    ("Oldest", SearchSort::Oldest),
                    ("Most Relevant", SearchSort::Relevance),
                ]
                .into_iter()
                .fold(menu, |menu, (label, option)| {
                    let screen = screen.clone();
                    menu.item(PopupMenuItem::new(label).checked(option == sort).on_click(
                        move |_, _, cx| {
                            if let Some(screen) = screen.upgrade() {
                                screen.update(cx, |this, cx| this.set_search_sort(option, cx));
                            }
                        },
                    ))
                })
            });
        let header = h_flex()
            .h(px(PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_semibold()
                    .child(title),
            )
            .child(sort_menu);

        let pages = panel.page_count();
        let footer = (pages > 1 && panel.error.is_none()).then(|| {
            let page = panel.page;
            h_flex()
                .h(px(44.))
                .flex_shrink_0()
                .px_3()
                .gap_2()
                .items_center()
                .justify_between()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Button::new("search-previous-page")
                        .label("Previous")
                        .outline()
                        .small()
                        .disabled(page == 0 || panel.loading)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_search_page(page.saturating_sub(1), cx)
                        })),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Page")
                        .child(
                            div().w(px(PAGE_BOX_WIDTH)).child(
                                Input::new(&self.search_page_input)
                                    .small()
                                    .disabled(panel.loading),
                            ),
                        )
                        .child(format!("of {pages}")),
                )
                .child(
                    Button::new("search-next-page")
                        .label("Next")
                        .outline()
                        .small()
                        .disabled(page + 1 >= pages || panel.loading)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_search_page(page + 1, cx)),
                        ),
                )
        });

        Some(
            v_flex()
                .w(px(CARD_LIST_WIDTH))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.sidebar)
                .child(header)
                .child(body)
                .children(footer),
        )
    }

    /// The list can't pad or space items, so each row carries the gap above it,
    /// and the last the one below too. In a server, a hit in a different channel
    /// from the one above is headed by its channel's name.
    fn render_search_hit(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let Some(panel) = &self.search else {
            return div().into_any_element();
        };
        let Some(hit) = panel.hits.get(ix) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let new_channel = matches!(panel.scope, SearchScope::Guild(_))
            && ix
                .checked_sub(1)
                .and_then(|prev| panel.hits.get(prev))
                .is_none_or(|prev| prev.channel_id != hit.channel_id);
        let channel = new_channel
            .then(|| self.channel_info(hit.channel_id))
            .flatten()
            .map(|channel| {
                h_flex()
                    .pt_1()
                    .pb(px(LIST_GAP / 2.))
                    .gap_1()
                    .items_center()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::default()
                            .path(channel_icon_path(channel.kind))
                            .small(),
                    )
                    .child(div().truncate().child(channel.name.clone()))
            });

        let group: SharedString = format!("search-hit-{ix}").into();
        let card = self
            .render_message_card(&hit.message, "search", cx)
            .group(group.clone())
            .hover(|style| style.bg(theme.muted.opacity(0.3)))
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .child(
                        Button::new(("search-jump", ix))
                            .label("Jump")
                            .outline()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.jump_to_search_hit(ix, window, cx)
                            })),
                    ),
            );

        div()
            .px(px(LIST_GAP))
            .pt(px(LIST_GAP))
            .when(ix + 1 == panel.hits.len(), |this| this.pb(px(LIST_GAP)))
            .children(channel)
            .child(card)
            .into_any_element()
    }
}
