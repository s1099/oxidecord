//! The emoji picker: opening it, choosing from it, and deciding which custom
//! emoji the user is allowed to send where they're writing.
//!
//! The rules follow Discord's: without Nitro, only the open guild's own static
//! emoji; with it, animated ones and every guild's, the latter only where the
//! channel grants `USE_EXTERNAL_EMOJIS`. Unicode emoji are always free.

use std::rc::Rc;

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::GuildEmoji;
use crate::screens::home::emoji::{
    PickerEmoji, PickerSection, emoji_names, picker_rows, resolve_custom_emoji,
};
use crate::screens::home::{EmojiPicker, HomeScreen, View};

/// Whether a custom emoji can be sent in the open conversation.
enum Access {
    Usable,
    /// Shown in the picker but can't be chosen, with the reason why.
    Locked(&'static str),
    /// Not offered at all.
    Hidden,
}

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_emoji_picker(
        &mut self,
        position: Point<Pixels>,
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
        self.emoji_picker = Some(EmojiPicker {
            position,
            search,
            rows: Rc::default(),
            hovered: None,
            scroll: UniformListScrollHandle::new(),
            _search_changed: search_changed,
        });
        self.refresh_emoji_picker(cx);
    }

    /// Closes the picker and hands the keyboard back to the composer.
    pub(in crate::screens::home) fn close_emoji_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.emoji_picker.take().is_some() {
            self.message_input.focus_handle(cx).focus(window);
            cx.notify();
        }
    }

    /// Lays the picker's list out again for the current search.
    fn refresh_emoji_picker(&mut self, cx: &mut Context<Self>) {
        let custom = self.custom_emoji_sections();
        let Some(picker) = &mut self.emoji_picker else {
            return;
        };
        let query = picker.search.read(cx).value();
        picker.rows = Rc::new(picker_rows(custom, &query));
        picker.hovered = None;
        picker.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Puts the chosen emoji into the composer at the cursor. Shift keeps the
    /// picker open for choosing several, as in Discord.
    pub(in crate::screens::home) fn pick_emoji(
        &mut self,
        emoji: &PickerEmoji,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = match emoji {
            PickerEmoji::Unicode(emoji) => emoji.as_str().to_string(),
            PickerEmoji::Custom {
                locked: Some(_), ..
            } => return,
            // The readable `:name:` when sending will turn it back into this
            // emoji; markup when another one of the same name would win.
            PickerEmoji::Custom { emoji, .. } => {
                let usable = self.usable_custom_emoji();
                match emoji_names(usable).get(emoji.name.as_str()) {
                    Some(winner) if winner.id == emoji.id => format!(":{}:", emoji.name),
                    _ => emoji.markdown(),
                }
            }
        };
        self.message_input.update(cx, |input, cx| {
            input.insert(text, window, cx);
        });
        if !window.modifiers().shift {
            self.close_emoji_picker(window, cx);
        }
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

    /// `text` with every `:name:` of an emoji the user may send here turned
    /// into the markup Discord renders.
    pub(in crate::screens::home) fn resolve_emoji_names(&self, text: &str) -> String {
        resolve_custom_emoji(text, &emoji_names(self.usable_custom_emoji()))
    }

    /// The guild whose emoji count as local: the open one, unless the
    /// conversation is a DM, where every guild's are external.
    fn local_guild(&self) -> Option<Id<GuildMarker>> {
        self.selected_guild.filter(|_| self.view == View::Guild)
    }

    /// Every guild's emoji in picker order: the open guild first, then the
    /// rest as the guild list has them.
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

    fn usable_custom_emoji(&self) -> impl Iterator<Item = &GuildEmoji> {
        self.emoji_by_guild().flat_map(move |(guild_id, emojis)| {
            emojis
                .iter()
                .filter(move |emoji| matches!(self.emoji_access(guild_id, emoji), Access::Usable))
        })
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
                    title: guild.name.clone(),
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
            let held = self.self_roles.get(&guild_id);
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

        // Another guild's: Nitro, and a channel that allows them. Without
        // either they're left out, rather than filling the picker with
        // whole guilds of emoji that can't be sent.
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
