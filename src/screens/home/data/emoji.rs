//! The emoji picker and the rules for which custom emoji the user may send where
//! they're writing.
//!
//! Without Nitro: only the open guild's static emoji. With it: animated ones and
//! every guild's, the latter only where the channel grants `USE_EXTERNAL_EMOJIS`.
//! Unicode emoji are always free.

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::{self, GuildEmoji};
use crate::platform::prefs::{self, RecentEmoji};
use crate::screens::home::emoji::{
    PickerEmoji, PickerSection, RECENT_LIMIT, SectionKind, emoji_names, picker_rows,
    resolve_custom_emoji,
};
use crate::screens::home::{EmojiPicker, EmojiTarget, HomeScreen, View};
use crate::ui::smooth_scroll::SmoothScroll;

enum Access {
    Usable,
    Locked(&'static str),
    Hidden,
}

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_emoji_picker(
        &mut self,
        position: Point<Pixels>,
        target: EmojiTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.emoji_picker.is_some() {
            return self.close_emoji_picker(window, cx);
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find the perfect emoji"));
        let search_changed = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_emoji_picker(cx);
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        let recent = self.recent_emojis();
        self.emoji_picker = Some(EmojiPicker {
            position,
            target,
            search,
            recent,
            layout: Rc::default(),
            collapsed: HashSet::new(),
            hovered: None,
            scroll: SmoothScroll::uniform_list(),
            rail_scroll: ScrollHandle::new(),
            _search_changed: search_changed,
        });
        self.refresh_emoji_picker(cx);
    }

    pub(in crate::screens::home) fn close_emoji_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.emoji_picker.take().is_some() {
            self.message_input.focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }

    fn refresh_emoji_picker(&mut self, cx: &mut Context<Self>) {
        self.relayout_emoji_picker(cx);
        if let Some(picker) = &mut self.emoji_picker {
            picker.hovered = None;
            picker.scroll.jump_to(px(0.));
        }
    }

    fn relayout_emoji_picker(&mut self, cx: &mut Context<Self>) {
        let custom = self.custom_emoji_sections();
        let Some(picker) = &mut self.emoji_picker else {
            return;
        };
        let query = picker.search.read(cx).value();
        picker.layout = Rc::new(picker_rows(
            picker.recent.clone(),
            custom,
            &query,
            &picker.collapsed,
        ));
        cx.notify();
    }

    pub(in crate::screens::home) fn toggle_picker_section(
        &mut self,
        kind: SectionKind,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = &mut self.emoji_picker
            && !picker.collapsed.remove(&kind)
        {
            picker.collapsed.insert(kind);
        }
        self.relayout_emoji_picker(cx);
    }

    /// Shift keeps the picker open for several picks, as in Discord.
    pub(in crate::screens::home) fn pick_emoji(
        &mut self,
        emoji: &PickerEmoji,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.emoji_picker.as_ref().map(|picker| picker.target) else {
            return;
        };
        match target {
            EmojiTarget::Composer => self.insert_emoji(emoji, window, cx),
            EmojiTarget::Reaction(message_id) => {
                let reaction = match emoji {
                    PickerEmoji::Unicode(emoji) => {
                        discord::ReactionEmoji::Unicode(emoji.as_str().to_string())
                    }
                    PickerEmoji::Custom {
                        locked: Some(_), ..
                    } => return,
                    PickerEmoji::Custom { emoji, .. } => discord::ReactionEmoji::Custom {
                        id: emoji.id,
                        name: emoji.name.clone(),
                        animated: emoji.animated,
                    },
                };
                self.add_reaction(message_id, reaction, cx);
            }
        }
        if !matches!(
            emoji,
            PickerEmoji::Custom {
                locked: Some(_),
                ..
            }
        ) {
            remember_emoji(emoji.recent_key());
        }
        if !window.modifiers().shift {
            self.close_emoji_picker(window, cx);
        }
    }

    fn insert_emoji(&mut self, emoji: &PickerEmoji, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.composer_emoji_text(emoji) else {
            return;
        };
        self.message_input.update(cx, |input, cx| {
            input.insert(text, window, cx);
        });
    }

    pub(in crate::screens::home) fn composer_emoji_text(
        &self,
        emoji: &PickerEmoji,
    ) -> Option<String> {
        Some(match emoji {
            PickerEmoji::Unicode(emoji) => emoji.as_str().to_string(),
            PickerEmoji::Custom {
                locked: Some(_), ..
            } => return None,
            // `:name:` is turned back into this emoji on send; markup when another emoji
            // of the same name would win.
            PickerEmoji::Custom { emoji, .. } => {
                let usable = self.usable_custom_emoji();
                match emoji_names(usable).get(emoji.name.as_str()) {
                    Some(winner) if winner.id == emoji.id => format!(":{}:", emoji.name),
                    _ => emoji.markdown(),
                }
            }
        })
    }

    pub(in crate::screens::home) fn hover_emoji(
        &mut self,
        emoji: Option<PickerEmoji>,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = &mut self.emoji_picker {
            picker.hovered = emoji;
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn resolve_emoji_names(&self, text: &str) -> String {
        resolve_custom_emoji(text, &emoji_names(self.usable_custom_emoji()))
    }

    /// The open guild; in a DM none, so every guild's emoji are external.
    fn local_guild(&self) -> Option<Id<GuildMarker>> {
        self.selected_guild.filter(|_| self.view == View::Guild)
    }

    fn emoji_by_guild(&self) -> impl Iterator<Item = (Id<GuildMarker>, &[GuildEmoji])> {
        let local = self.local_guild();
        let rest = self
            .guilds
            .iter()
            .map(|guild| guild.id)
            .filter(move |id| Some(*id) != local);
        local.into_iter().chain(rest).filter_map(|guild_id| {
            let emojis = self.guild_emojis.get(&guild_id)?;
            Some((guild_id, emojis.as_slice()))
        })
    }

    pub(in crate::screens::home) fn usable_custom_emoji(
        &self,
    ) -> impl Iterator<Item = &GuildEmoji> {
        self.emoji_by_guild().flat_map(move |(guild_id, emojis)| {
            emojis
                .iter()
                .filter(move |emoji| matches!(self.emoji_access(guild_id, emoji), Access::Usable))
        })
    }

    fn recent_emojis(&self) -> Vec<PickerEmoji> {
        let custom: Vec<_> = self
            .custom_emoji_sections()
            .into_iter()
            .flat_map(|section| section.emojis)
            .collect();
        prefs::load()
            .recent_emojis
            .iter()
            .filter_map(|recent| match recent {
                RecentEmoji::Unicode(text) => emojis::get(text).map(PickerEmoji::Unicode),
                RecentEmoji::Custom(id) => custom
                    .iter()
                    .find(|emoji| emoji.recent_key() == RecentEmoji::Custom(*id))
                    .cloned(),
            })
            .take(RECENT_LIMIT)
            .collect()
    }

    fn custom_emoji_sections(&self) -> Vec<PickerSection> {
        self.emoji_by_guild()
            .filter_map(|(guild_id, emojis)| {
                let emojis: Vec<_> = emojis
                    .iter()
                    .filter_map(|emoji| {
                        let locked = match self.emoji_access(guild_id, emoji) {
                            Access::Usable => None,
                            Access::Locked(reason) => Some(reason),
                            Access::Hidden => return None,
                        };
                        Some(PickerEmoji::Custom {
                            emoji: emoji.clone(),
                            locked,
                        })
                    })
                    .collect();
                let guild = self.guilds.iter().find(|guild| guild.id == guild_id)?;
                (!emojis.is_empty()).then(|| PickerSection {
                    kind: SectionKind::Guild(guild_id),
                    title: guild.name.clone(),
                    icon: guild.icon_url.clone(),
                    emojis,
                })
            })
            .collect()
    }

    fn emoji_access(&self, guild_id: Id<GuildMarker>, emoji: &GuildEmoji) -> Access {
        if !emoji.available {
            return Access::Hidden;
        }
        // Restricted to roles the user doesn't hold: Discord doesn't list
        // these at all.
        if !emoji.roles.is_empty() {
            let held = self.self_roles(guild_id);
            if !held.is_some_and(|held| emoji.roles.iter().any(|role| held.contains(role))) {
                return Access::Hidden;
            }
        }

        let premium = self.current_user.as_ref().is_some_and(|user| user.premium);
        if Some(guild_id) == self.local_guild() {
            return if emoji.animated && !premium {
                Access::Locked("Animated emoji require Nitro")
            } else {
                Access::Usable
            };
        }

        // Another guild's emoji need Nitro and a channel that allows them; otherwise
        // they're left out rather than filling the picker with unsendable ones.
        let channel_allows = self.view == View::DirectMessages
            || self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_use_external_emoji);
        if premium && channel_allows {
            Access::Usable
        } else {
            Access::Hidden
        }
    }
}

/// The open picker keeps its snapshot; the next one shows the change.
pub(super) fn remember_emoji(emoji: RecentEmoji) {
    prefs::update(|prefs| {
        let recent = &mut prefs.recent_emojis;
        recent.retain(|other| *other != emoji);
        recent.insert(0, emoji);
        recent.truncate(RECENT_LIMIT);
    });
}
