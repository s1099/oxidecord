//! The composer's autocomplete: what's offered for the `@`, `#` or `:` word at
//! the cursor, moving through it from the keyboard, and writing the chosen
//! one into the composer.
//!
//! Members are offered from whoever the app already knows of in the
//! conversation, and as each name is typed the gateway is searched for the
//! rest of the guild; the answer refreshes the list when it lands.

use std::collections::HashSet;

use gpui_kit::component::input::Escape;
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{EmojiMarker, GuildMarker, UserMarker},
};

use crate::discord::{self, ChannelKind};
use crate::screens::home::autocomplete::{
    Broadcast, LIMIT, Suggestion, Trigger, best, match_rank, word_at,
};
use crate::screens::home::emoji::{PickerEmoji, unicode_emojis};
use crate::screens::home::{
    Autocomplete, AutocompleteAccept, AutocompleteNext, AutocompletePrevious, HomeScreen, View,
};

use super::emoji::remember_emoji;

impl HomeScreen {
    /// Called on every change to the composer's text, and when the gateway
    /// names more members: offers suggestions for the word at the cursor, or
    /// takes them away.
    pub(in crate::screens::home) fn refresh_autocomplete(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let input = self.message_input.read(cx);
            (input.value(), input.cursor())
        };
        let Some(word) = word_at(&text, cursor) else {
            self.autocomplete_dismissed = None;
            return self.close_autocomplete(cx);
        };
        if self.autocomplete_dismissed == Some(word.range.start) {
            return self.close_autocomplete(cx);
        }
        self.autocomplete_dismissed = None;

        let query = word.query.to_lowercase();
        let items = match word.trigger {
            Trigger::User => {
                self.search_members(&query);
                self.user_suggestions(&query)
            }
            Trigger::Channel => self.channel_suggestions(&query),
            Trigger::Emoji => self.emoji_suggestions(&query),
        };
        if items.is_empty() {
            return self.close_autocomplete(cx);
        }

        // Rebuilt under the highlight as members arrive or the word grows, so
        // the highlight follows what it was on rather than its position.
        let selected = self
            .autocomplete
            .as_ref()
            .filter(|old| old.trigger == word.trigger && old.range.start == word.range.start)
            .and_then(|old| old.items.get(old.selected))
            .and_then(|old| items.iter().position(|item| item.key() == old.key()))
            .unwrap_or(0);
        self.autocomplete = Some(Autocomplete {
            trigger: word.trigger,
            range: word.range,
            query: word.query.to_string(),
            items,
            selected,
        });
        cx.notify();
    }

    fn close_autocomplete(&mut self, cx: &mut Context<Self>) {
        if self.autocomplete.take().is_some() {
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn on_autocomplete_previous(
        &mut self,
        _: &AutocompletePrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_autocomplete(-1, cx);
    }

    pub(in crate::screens::home) fn on_autocomplete_next(
        &mut self,
        _: &AutocompleteNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_autocomplete(1, cx);
    }

    /// Without suggestions showing, the arrow goes on to its other bindings:
    /// editing the last message, or moving the cursor.
    fn step_autocomplete(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(autocomplete) = &mut self.autocomplete else {
            cx.propagate();
            return;
        };
        let len = autocomplete.items.len() as isize;
        autocomplete.selected = (autocomplete.selected as isize + step).rem_euclid(len) as usize;
        cx.notify();
    }

    pub(in crate::screens::home) fn on_autocomplete_accept(
        &mut self,
        _: &AutocompleteAccept,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .autocomplete
            .as_ref()
            .map(|autocomplete| autocomplete.selected)
        {
            Some(selected) => self.accept_suggestion(selected, window, cx),
            // Tab indents, as it would anywhere else in the input.
            None => cx.propagate(),
        }
    }

    /// Escape puts the suggestions away until the cursor leaves the word.
    /// With none showing it drops the reply being composed, and after that
    /// reads the conversation, as Discord's does.
    pub(in crate::screens::home) fn on_autocomplete_escape(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The viewer covers the composer, so it's what escape is for.
        if self.image_viewer.is_some() {
            self.close_image_viewer(window, cx);
        } else if let Some(autocomplete) = self.autocomplete.take() {
            self.autocomplete_dismissed = Some(autocomplete.range.start);
            cx.notify();
        } else if self.replying_to.take().is_some() {
            cx.notify();
        } else {
            self.mark_conversation_read(window, cx);
        }
    }

    pub(in crate::screens::home) fn hover_suggestion(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(autocomplete) = &mut self.autocomplete
            && autocomplete.selected != index
        {
            autocomplete.selected = index;
            cx.notify();
        }
    }

    /// Replaces the word being completed with the suggestion at `index`, and
    /// a space to carry on typing after.
    pub(in crate::screens::home) fn accept_suggestion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(autocomplete) = self.autocomplete.take() else {
            return;
        };
        let Some(suggestion) = autocomplete.items.get(index) else {
            return;
        };
        // Mentions go in readable and are swapped for their markup on send,
        // the way Discord's composer shows a name rather than an id.
        let text = match suggestion {
            Suggestion::User { id, name, .. } => {
                self.remember_mention(format!("@{name}"), format!("<@{id}>"))
            }
            Suggestion::Channel { id, name, .. } => {
                self.remember_mention(format!("#{name}"), format!("<#{id}>"))
            }
            Suggestion::Broadcast(broadcast) => broadcast.text().to_string(),
            Suggestion::Emoji { emoji, .. } => {
                let Some(text) = self.composer_emoji_text(emoji) else {
                    return;
                };
                remember_emoji(emoji.recent_key());
                text
            }
        };
        let range = autocomplete.range;
        self.message_input.update(cx, |input, cx| {
            let value = input.value();
            let (Some(before), Some(word)) = (value.get(..range.start), value.get(range.clone()))
            else {
                return;
            };
            // The input takes ranges in UTF-16, as platform text input does.
            let start = before.encode_utf16().count();
            let end = start + word.encode_utf16().count();
            input.replace_text_in_range(Some(start..end), &format!("{text} "), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn remember_mention(&mut self, shown: String, markup: String) -> String {
        self.composer_mentions.retain(|(other, _)| *other != shown);
        self.composer_mentions.push((shown.clone(), markup));
        shown
    }

    /// Asks the gateway for the open guild's members whose names start with
    /// `query`, once per query a session. A user session only knows a sliver
    /// of a guild's members, so without this only people already seen in the
    /// conversation could be mentioned.
    fn search_members(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        let Some(guild_id) = self.selected_guild.filter(|_| self.view == View::Guild) else {
            return;
        };
        let Some(gateway) = &self.gateway else {
            return;
        };
        if self.member_searches.insert((guild_id, query.to_string())) {
            gateway.search_members(guild_id, query, LIMIT as u8);
        }
    }

    /// Takes the members a chunk names, and offers them if they finish the
    /// word being typed.
    pub(in crate::screens::home) fn add_guild_members(
        &mut self,
        guild_id: Id<GuildMarker>,
        members: Vec<discord::GuildMember>,
        cx: &mut Context<Self>,
    ) {
        let known = self.guild_members.entry(guild_id).or_default();
        let mut changed = false;
        for member in members {
            // Every live message names its author again, mostly unchanged,
            // which isn't worth recomputing the suggestions over.
            changed |= known
                .get(&member.user_id)
                .is_none_or(|known| known.name != member.name);
            known.insert(member.user_id, member);
        }
        if changed
            && self.selected_guild == Some(guild_id)
            && self
                .autocomplete
                .as_ref()
                .is_none_or(|autocomplete| autocomplete.trigger == Trigger::User)
        {
            self.refresh_autocomplete(cx);
        }
    }

    /// The people in the conversation matching `query`: those who've spoken
    /// most recently first, then the member list, then whoever else the
    /// gateway has named.
    fn user_suggestions(&self, query: &str) -> Vec<Suggestion> {
        let guild_id = self.selected_guild.filter(|_| self.view == View::Guild);
        let known = guild_id.and_then(|guild_id| self.guild_members.get(&guild_id));

        let mut seen = HashSet::new();
        let mut ranked = Vec::new();
        let mut offer = |id: Id<UserMarker>, name: &str, avatar_url: Option<&String>| {
            if !seen.insert(id) {
                return;
            }
            let username = known
                .and_then(|known| known.get(&id))
                .and_then(|member| member.username.clone());
            let rank = [Some(name), username.as_deref()]
                .into_iter()
                .flatten()
                .filter_map(|name| match_rank(name, query))
                .min();
            if let Some(rank) = rank {
                ranked.push((
                    rank,
                    Suggestion::User {
                        id,
                        name: name.to_string(),
                        username,
                        avatar_url: avatar_url.cloned(),
                    },
                ));
            }
        };

        for message in self.messages.iter().rev() {
            offer(
                message.author_id,
                &message.author_name,
                message.author_avatar_url.as_ref(),
            );
            for user in &message.mentions {
                offer(user.id, &user.name, user.avatar_url.as_ref());
            }
        }
        match guild_id {
            Some(guild_id) => {
                let list = self
                    .member_list
                    .as_ref()
                    .filter(|list| list.guild_id == guild_id);
                for row in list.iter().flat_map(|list| &list.rows) {
                    if let Some(discord::MemberListItem::Member(member)) = row {
                        offer(member.user_id, &member.name, member.avatar_url.as_ref());
                    }
                }
                for member in known.into_iter().flat_map(|known| known.values()) {
                    offer(member.user_id, &member.name, member.avatar_url.as_ref());
                }
            }
            None => {
                let dm = self
                    .dms
                    .iter()
                    .find(|dm| Some(dm.id) == self.selected_channel);
                if let Some(dm) = dm
                    && let Some(recipient) = dm.recipient_id
                {
                    offer(recipient, &dm.name, dm.avatar_url.as_ref());
                }
            }
        }
        if let Some(user) = &self.current_user {
            offer(user.id, &user.name, user.avatar_url.as_ref());
        }
        let mut suggestions = best(ranked);
        suggestions.extend(self.broadcast_suggestions(query));
        suggestions
    }

    /// `@everyone` and `@here`, below the members as in Discord, where the
    /// user may use them: in a guild channel that grants `MENTION_EVERYONE`.
    fn broadcast_suggestions(&self, query: &str) -> Vec<Suggestion> {
        let allowed = self.view == View::Guild
            && self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_mention_everyone);
        if !allowed {
            return Vec::new();
        }
        Broadcast::ALL
            .into_iter()
            .filter(|broadcast| match_rank(&broadcast.text()[1..], query).is_some())
            .map(Suggestion::Broadcast)
            .collect()
    }

    /// The open guild's channels matching `query`, in sidebar order. None in
    /// a DM, which has no channels to link.
    fn channel_suggestions(&self, query: &str) -> Vec<Suggestion> {
        if self.view != View::Guild {
            return Vec::new();
        }
        best(
            self.channel_groups
                .iter()
                .flat_map(|group| {
                    let category = group.category.as_ref().map(|category| &category.name);
                    group
                        .channels
                        .iter()
                        .map(move |channel| (channel, category))
                })
                .filter(|(channel, _)| channel.kind != ChannelKind::Category)
                .filter_map(|(channel, category)| {
                    Some((
                        match_rank(&channel.name, query)?,
                        Suggestion::Channel {
                            id: channel.id,
                            name: channel.name.clone(),
                            kind: channel.kind,
                            category: category.cloned(),
                        },
                    ))
                }),
        )
    }

    /// The emoji the user may send here matching `query`: custom ones by
    /// name, then unicode ones by shortcode.
    fn emoji_suggestions(&self, query: &str) -> Vec<Suggestion> {
        let custom = self.usable_custom_emoji().filter_map(|emoji| {
            Some((
                match_rank(&emoji.name, query)?,
                PickerEmoji::Custom {
                    emoji: emoji.clone(),
                    locked: None,
                },
            ))
        });
        let unicode = unicode_emojis().filter_map(|emoji| {
            let rank = emoji
                .shortcodes()
                .filter_map(|code| match_rank(code, query))
                .min()?;
            Some((rank, PickerEmoji::Unicode(emoji)))
        });
        best(custom.chain(unicode))
            .into_iter()
            .map(|emoji| {
                let guild = match &emoji {
                    PickerEmoji::Custom { emoji, .. } => self.emoji_guild_name(emoji.id),
                    PickerEmoji::Unicode(_) => None,
                };
                Suggestion::Emoji { emoji, guild }
            })
            .collect()
    }

    fn emoji_guild_name(&self, emoji_id: Id<EmojiMarker>) -> Option<String> {
        let (guild_id, _) = self
            .guild_emojis
            .iter()
            .find(|(_, emojis)| emojis.iter().any(|emoji| emoji.id == emoji_id))?;
        let guild = self.guilds.iter().find(|guild| guild.id == *guild_id)?;
        Some(guild.name.clone())
    }
}
