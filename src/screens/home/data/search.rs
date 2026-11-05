//! Searching messages from the conversation header: the suggestions under the
//! box as a query is typed, running it, paging and sorting the results, and
//! jumping to one.

use std::time::Duration;

use chrono::Local;
use gpui_kit::component::input::Escape;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, UserMarker},
};

use crate::discord::{
    self, AuthorType, ChannelKind, MessageLink, SEARCH_PAGE_SIZE, SearchFilters, SearchHas,
    SearchResponse, SearchScope, SearchSort,
};
use crate::screens::home::autocomplete::{LIMIT, Suggestion, match_rank};
use crate::screens::home::search::{
    Editing, FilterKind, SearchPanel, SearchSuggestion, SearchSuggestions, editing_at, filter_text,
    parse,
};
use crate::screens::home::{
    HomeScreen, SearchAccept, SearchNext, SearchPrevious, SearchSubmit, View,
};

/// Floor on the wait before asking again while Discord indexes, since it may
/// say zero.
const INDEXING_RETRY: Duration = Duration::from_secs(2);

impl HomeScreen {
    /// A guild's search covers all of it; a DM's, the conversation.
    pub(in crate::screens::home) fn search_scope(&self) -> Option<SearchScope> {
        match self.view {
            View::Guild => self.selected_guild.map(SearchScope::Guild),
            View::DirectMessages => self.selected_channel.map(SearchScope::Channel),
        }
    }

    /// Results belong to where they were searched, so leaving closes them and
    /// clears the box, as in Discord.
    pub(in crate::screens::home) fn sync_search_scope(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &self.search else {
            return;
        };
        if Some(panel.scope) != self.search_scope() {
            self.search = None;
            self.search_suggestions = None;
            self.search_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            return;
        }
        // Shows the page being viewed whenever it isn't being typed over.
        let shown = (panel.page + 1).to_string();
        if !self.search_page_editing && self.search_page_input.read(cx).value() != shown {
            self.search_page_input
                .update(cx, |input, cx| input.set_value(shown, window, cx));
        }
    }

    /// Enter in the footer's page box. Out of range clamps to the nearest page;
    /// nothing typed goes nowhere.
    pub(in crate::screens::home) fn submit_search_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typed = self
            .search_page_input
            .read(cx)
            .value()
            .trim()
            .parse::<usize>();
        if let (Ok(typed), Some(panel)) = (typed, &self.search) {
            let page = typed.clamp(1, panel.page_count()) - 1;
            self.set_search_page(page, cx);
        }
        // Letting go puts the current page back in the box.
        self.focus_handle.focus(window, cx);
    }

    pub(in crate::screens::home) fn on_search_focus(
        &mut self,
        focused: bool,
        cx: &mut Context<Self>,
    ) {
        self.search_focused = focused;
        if focused {
            self.refresh_search_suggestions(cx);
        } else if self.search_suggestions.take().is_some() {
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn focus_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Runs on every change to the box's text while it has focus.
    pub(in crate::screens::home) fn refresh_search_suggestions(&mut self, cx: &mut Context<Self>) {
        if !self.search_focused {
            return;
        }
        let (text, cursor) = {
            let input = self.search_input.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let in_dms = self.view == View::DirectMessages;
        let suggestions = match editing_at(&text, cursor) {
            Some(Editing::Key { prefix, range }) => {
                let prefix = prefix.to_lowercase();
                let items: Vec<_> = FilterKind::ALL
                    .into_iter()
                    .filter(|kind| !in_dms || kind.in_dms())
                    .filter(|kind| kind.key().to_lowercase().starts_with(&prefix))
                    .map(SearchSuggestion::Key)
                    .collect();
                // Nothing highlighted, so enter after a word like "in" searches for
                // it; tab takes the first.
                (!items.is_empty()).then_some(SearchSuggestions {
                    selected: None,
                    range,
                    kind: None,
                    items,
                })
            }
            Some(Editing::Value { kind, value, range }) => {
                let items = self.value_suggestions(kind, value);
                (!items.is_empty()).then_some(SearchSuggestions {
                    range,
                    kind: Some(kind),
                    items,
                    selected: Some(0),
                })
            }
            None => None,
        };
        // Rebuilt under the highlight so it follows the item, not the position.
        let suggestions = suggestions.map(|mut new| {
            if let Some(old) = &self.search_suggestions
                && old.range.start == new.range.start
                && let Some(key) = old
                    .selected
                    .and_then(|ix| old.items.get(ix))
                    .map(|s| s.key())
                && let Some(ix) = new.items.iter().position(|item| item.key() == key)
            {
                new.selected = Some(ix);
            }
            new
        });
        self.search_suggestions = suggestions;
        cx.notify();
    }

    fn value_suggestions(&mut self, kind: FilterKind, value: &str) -> Vec<SearchSuggestion> {
        let query = value.to_lowercase();
        let fixed = |values: &[(&str, Option<&'static str>)]| {
            values
                .iter()
                .filter(|(value, _)| match_rank(value, &query).is_some())
                .map(|&(value, detail)| SearchSuggestion::Value {
                    kind,
                    value: value.to_string(),
                    detail,
                })
                .collect()
        };
        match kind {
            FilterKind::From | FilterKind::Mentions => {
                self.search_guild_members(&query);
                self.user_suggestions(&query)
                    .into_iter()
                    .filter(|suggestion| matches!(suggestion, Suggestion::User { .. }))
                    .map(|suggestion| SearchSuggestion::Name(kind, suggestion))
                    .collect()
            }
            FilterKind::In => self
                .channel_suggestions(&query)
                .into_iter()
                .filter(|suggestion| {
                    matches!(suggestion, Suggestion::Channel { kind, .. } if *kind != ChannelKind::Forum)
                })
                .map(|suggestion| SearchSuggestion::Name(kind, suggestion))
                .collect(),
            FilterKind::Has => fixed(
                &SearchHas::ALL
                    .map(|has| (has.keyword(), None))
                    .into_iter()
                    .collect::<Vec<_>>(),
            ),
            FilterKind::AuthorType => fixed(
                &AuthorType::ALL
                    .map(|kind| (kind.keyword(), None))
                    .into_iter()
                    .collect::<Vec<_>>(),
            ),
            FilterKind::Pinned => fixed(&[("true", None), ("false", None)]),
            FilterKind::Before | FilterKind::During | FilterKind::After => {
                let today = Local::now().date_naive();
                let today_text = today.format("%Y-%m-%d").to_string();
                let yesterday_text = today
                    .pred_opt()
                    .map(|day| day.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                let mut items: Vec<SearchSuggestion> = [
                    (today_text, Some("Today")),
                    (yesterday_text, Some("Yesterday")),
                ]
                .into_iter()
                .filter(|(text, _)| text.starts_with(&query))
                .map(|(value, detail)| SearchSuggestion::Value {
                    kind,
                    value,
                    detail,
                })
                .collect();
                items.truncate(LIMIT);
                items
            }
        }
    }

    pub(in crate::screens::home) fn on_search_previous(
        &mut self,
        _: &SearchPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_search_suggestion(-1, cx);
    }

    pub(in crate::screens::home) fn on_search_next(
        &mut self,
        _: &SearchNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_search_suggestion(1, cx);
    }

    fn step_search_suggestion(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(suggestions) = &mut self.search_suggestions else {
            cx.propagate();
            return;
        };
        let len = suggestions.items.len() as isize;
        let next = match suggestions.selected {
            Some(ix) => ix as isize + step,
            // Down from nothing lands on the first, up on the last.
            None if step > 0 => 0,
            None => len - 1,
        };
        suggestions.selected = Some(next.rem_euclid(len) as usize);
        cx.notify();
    }

    pub(in crate::screens::home) fn hover_search_suggestion(
        &mut self,
        ix: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(suggestions) = &mut self.search_suggestions
            && suggestions.selected != Some(ix)
        {
            suggestions.selected = Some(ix);
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn on_search_accept(
        &mut self,
        _: &SearchAccept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &self.search_suggestions {
            Some(suggestions) => {
                let ix = suggestions.selected.unwrap_or(0);
                self.accept_search_suggestion(ix, window, cx);
            }
            None => cx.propagate(),
        }
    }

    pub(in crate::screens::home) fn on_search_submit(
        &mut self,
        _: &SearchSubmit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .search_suggestions
            .as_ref()
            .and_then(|suggestions| suggestions.selected)
        {
            Some(ix) => self.accept_search_suggestion(ix, window, cx),
            None => self.run_search(window, cx),
        }
    }

    /// Closes suggestions, then clears the box, then lets go of it.
    pub(in crate::screens::home) fn on_search_escape(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_suggestions.take().is_some() {
            cx.notify();
        } else if !self.search_input.read(cx).value().is_empty() {
            self.search_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.refresh_search_suggestions(cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    pub(in crate::screens::home) fn accept_search_suggestion(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(suggestions) = self.search_suggestions.take() else {
            return;
        };
        let Some(suggestion) = suggestions.items.get(ix) else {
            return;
        };
        let text = match suggestion {
            SearchSuggestion::Key(kind) => format!("{}:", kind.key()),
            SearchSuggestion::Name(
                kind,
                Suggestion::User {
                    id, name, username, ..
                },
            ) => {
                // The username where known, being unique; the name otherwise.
                let shown = username.clone().unwrap_or_else(|| name.clone());
                self.search_names.users.insert(shown.to_lowercase(), *id);
                filter_text(*kind, &shown)
            }
            SearchSuggestion::Name(kind, Suggestion::Channel { id, name, .. }) => {
                self.search_names.channels.insert(name.to_lowercase(), *id);
                filter_text(*kind, name)
            }
            SearchSuggestion::Name(..) => return,
            SearchSuggestion::Value { kind, value, .. } => filter_text(*kind, value),
        };
        let range = suggestions.range;
        self.search_input.update(cx, |input, cx| {
            let value = input.value();
            let (Some(before), Some(word)) = (value.get(..range.start), value.get(range.clone()))
            else {
                return;
            };
            // The input takes ranges in UTF-16.
            let start = before.encode_utf16().count();
            let end = start + word.encode_utf16().count();
            input.replace_text_in_range(Some(start..end), &text, window, cx);
            input.focus(window, cx);
        });
        self.refresh_search_suggestions(cx);
    }

    /// Searches what's in the box, from the first page, newest first unless a
    /// sort was picked for the open results.
    pub(in crate::screens::home) fn run_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.search_scope() else {
            return;
        };
        let text = self.search_input.read(cx).value().to_string();
        let sort = self
            .search
            .as_ref()
            .filter(|panel| panel.scope == scope)
            .map(|panel| panel.filters.sort)
            .unwrap_or_default();
        self.search_suggestions = None;
        let filters = match self.resolve_search(&text, sort) {
            Ok(filters) if filters.is_empty() => {
                self.search = None;
                cx.notify();
                return;
            }
            Ok(filters) => filters,
            Err(error) => {
                self.search = Some(SearchPanel {
                    scope,
                    filters: SearchFilters::default(),
                    page: 0,
                    total: 0,
                    hits: Vec::new(),
                    list: ListState::new(0, ListAlignment::Top, px(256.)),
                    loading: false,
                    indexing: false,
                    error: Some(error),
                    _task: None,
                });
                cx.notify();
                return;
            }
        };
        self.search = Some(SearchPanel {
            scope,
            filters,
            page: 0,
            total: 0,
            hits: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(256.)),
            loading: true,
            indexing: false,
            error: None,
            _task: None,
        });
        // Lets go of the box, so the suggestions close and typing goes to the composer.
        self.focus_handle.focus(window, cx);
        self.fetch_search_page(cx);
    }

    fn resolve_search(&self, text: &str, sort: SearchSort) -> Result<SearchFilters, String> {
        let parsed = parse(text, Local::now().date_naive())?;
        let users = |names: &[String]| {
            names
                .iter()
                .map(|name| {
                    self.resolve_search_user(name)
                        .ok_or_else(|| format!("Couldn't find anyone called “{name}”."))
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let channels = parsed
            .channels
            .iter()
            .map(|name| {
                if self.view != View::Guild {
                    return Err("in: only works in a server.".to_string());
                }
                self.resolve_search_channel(name)
                    .ok_or_else(|| format!("Couldn't find a channel called “#{name}”."))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchFilters {
            authors: users(&parsed.from)?,
            mentions: users(&parsed.mentions)?,
            channels,
            content: parsed.content,
            has: parsed.has,
            author_types: parsed.author_types,
            min_id: parsed.min_id,
            max_id: parsed.max_id,
            pinned: parsed.pinned,
            sort,
        })
    }

    /// A picked suggestion first, then anyone known by that username or name.
    fn resolve_search_user(&self, name: &str) -> Option<Id<UserMarker>> {
        let lower = name.to_lowercase();
        if let Some(&id) = self.search_names.users.get(&lower) {
            return Some(id);
        }
        if let Some(user) = &self.current_user
            && (user.username.to_lowercase() == lower || user.name.to_lowercase() == lower)
        {
            return Some(user.id);
        }
        let matches = |candidate: Option<&String>| {
            candidate.is_some_and(|candidate| candidate.to_lowercase() == lower)
        };
        self.user_suggestions(&lower)
            .into_iter()
            .find_map(|suggestion| match suggestion {
                Suggestion::User {
                    id, name, username, ..
                } if matches(username.as_ref()) || matches(Some(&name)) => Some(id),
                _ => None,
            })
    }

    fn resolve_search_channel(&self, name: &str) -> Option<Id<ChannelMarker>> {
        let lower = name.to_lowercase();
        if let Some(&id) = self.search_names.channels.get(&lower) {
            return Some(id);
        }
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| {
                channel.kind != ChannelKind::Category && channel.name.to_lowercase() == lower
            })
            .map(|channel| channel.id)
    }

    pub(in crate::screens::home) fn set_search_sort(
        &mut self,
        sort: SearchSort,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &mut self.search else {
            return;
        };
        if panel.filters.sort == sort || panel.filters.is_empty() {
            return;
        }
        panel.filters.sort = sort;
        panel.page = 0;
        self.fetch_search_page(cx);
    }

    pub(in crate::screens::home) fn set_search_page(
        &mut self,
        page: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &mut self.search else {
            return;
        };
        if page == panel.page || page >= panel.page_count() {
            return;
        }
        panel.page = page;
        self.fetch_search_page(cx);
    }

    pub(in crate::screens::home) fn close_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search = None;
        self.search_suggestions = None;
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// The open panel's page, under its filters. Replaces any page in flight.
    fn fetch_search_page(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = &mut self.search else {
            return;
        };
        panel.loading = true;
        panel.error = None;
        let (scope, filters, offset) = (
            panel.scope,
            panel.filters.clone(),
            panel.page * SEARCH_PAGE_SIZE,
        );
        panel._task = Some(cx.spawn(async move |this, cx| {
            loop {
                let result = discord::search_messages(scope, filters.clone(), offset).await;
                let retry = this
                    .update(cx, |this, cx| {
                        let panel = this.search.as_mut()?;
                        match result {
                            Ok(SearchResponse::Results { total, hits }) => {
                                panel.loading = false;
                                panel.indexing = false;
                                panel.total = total;
                                panel.list.reset(hits.len());
                                panel.hits = hits;
                                // A guild's results name authors by their members.
                                this.request_author_members();
                                cx.notify();
                                None
                            }
                            Ok(SearchResponse::Indexing { retry_after }) => {
                                panel.indexing = true;
                                cx.notify();
                                Some(Duration::from_secs_f64(retry_after).max(INDEXING_RETRY))
                            }
                            Err(error) => {
                                panel.loading = false;
                                panel.indexing = false;
                                panel.error = Some(error);
                                cx.notify();
                                None
                            }
                        }
                    })
                    .ok()
                    .flatten();
                let Some(wait) = retry else {
                    return;
                };
                cx.background_executor().timer(wait).await;
            }
        }));
        cx.notify();
    }

    /// Opens the hit's conversation at it, leaving the results open.
    pub(in crate::screens::home) fn jump_to_search_hit(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = &self.search else {
            return;
        };
        let Some(hit) = panel.hits.get(ix) else {
            return;
        };
        let guild_id = match panel.scope {
            SearchScope::Guild(guild_id) => Some(guild_id),
            SearchScope::Channel(_) => None,
        };
        let link = MessageLink {
            guild_id,
            channel_id: hit.channel_id,
            message_id: hit.message.id,
        };
        if !self.open_message_link(link, window, cx) {
            let guild = guild_id.map_or("@me".to_string(), |id| id.to_string());
            cx.open_url(&format!(
                "https://discord.com/channels/{guild}/{}/{}",
                link.channel_id, link.message_id
            ));
        }
    }
}
