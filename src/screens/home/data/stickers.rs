//! The sticker picker and the rules for which stickers the user may send where
//! they're writing.
//!
//! Discord's own stickers are free to everyone, as are a guild's in that guild.
//! Another guild's need Nitro and, in a guild channel, `USE_EXTERNAL_STICKERS`;
//! without them they're left out, as the emoji picker leaves out such emoji.

use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use twilight_model::id::{Id, marker::GuildMarker};

use crate::discord::{self, Sticker};
use crate::platform::prefs;
use crate::screens::home::stickers::{
    PickerSection, RECENT_LIMIT, SectionIcon, SectionKind, picker_rows,
};
use crate::screens::home::{HomeScreen, StickerPicker, View};
use crate::ui::smooth_scroll::SmoothScroll;

impl HomeScreen {
    pub(in crate::screens::home) fn toggle_sticker_picker(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sticker_picker.is_some() {
            return self.close_sticker_picker(window, cx);
        }
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find the perfect sticker"));
        let search_changed = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_sticker_picker(cx);
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        let list = ListState::new(0, ListAlignment::Top, px(200.));
        self.sticker_picker = Some(StickerPicker {
            position,
            search,
            layout: Rc::default(),
            hovered: None,
            scroll: SmoothScroll::list(list.clone()),
            list,
            rail_scroll: ScrollHandle::new(),
            _search_changed: search_changed,
        });
        self.refresh_sticker_picker(cx);
        self.load_sticker_packs(cx);
    }

    pub(in crate::screens::home) fn close_sticker_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sticker_picker.take().is_some() {
            self.message_input.focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }

    fn refresh_sticker_picker(&mut self, cx: &mut Context<Self>) {
        let sections = self.sticker_sections();
        let Some(picker) = &mut self.sticker_picker else {
            return;
        };
        let query = picker.search.read(cx).value();
        picker.layout = Rc::new(picker_rows(sections, &query));
        picker.list.reset(picker.layout.rows.len());
        picker.hovered = None;
        picker.scroll.jump_to(px(0.));
        cx.notify();
    }

    fn load_sticker_packs(&mut self, cx: &mut Context<Self>) {
        let packs = &mut self.sticker_packs;
        if !packs.packs.is_empty() || packs.loading {
            return;
        }
        packs.loading = true;
        packs.error = None;
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_sticker_packs().await;
            let _ = this.update(cx, |this, cx| {
                this.sticker_packs.loading = false;
                match result {
                    Ok(packs) => this.sticker_packs.packs = packs,
                    Err(err) => this.sticker_packs.error = Some(err),
                }
                this.refresh_sticker_picker(cx);
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn hover_sticker(
        &mut self,
        sticker: Option<Sticker>,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = &mut self.sticker_picker
            && picker.hovered.as_ref().map(|s| s.item.id) != sticker.as_ref().map(|s| s.item.id)
        {
            picker.hovered = sticker;
            cx.notify();
        }
    }

    /// Sends at once, with whatever is typed, as Discord's client does. Shift keeps
    /// the picker open, as it does the emoji picker.
    pub(in crate::screens::home) fn pick_sticker(
        &mut self,
        sticker: &Sticker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        remember_sticker(sticker.item.id.get());
        if !window.modifiers().shift {
            self.close_sticker_picker(window, cx);
        }
        self.send_composed(vec![sticker.item.clone()], window, cx);
    }

    fn stickers_by_guild(&self) -> impl Iterator<Item = (Id<GuildMarker>, &[Sticker])> {
        let local = self.local_guild();
        let rest = self
            .guilds
            .iter()
            .map(|guild| guild.id)
            .filter(move |id| Some(*id) != local);
        local.into_iter().chain(rest).filter_map(|guild_id| {
            let stickers = self.guild_data.stickers.get(&guild_id)?;
            Some((guild_id, stickers.as_slice()))
        })
    }

    fn can_send_sticker(&self, guild_id: Id<GuildMarker>, sticker: &Sticker) -> bool {
        if !sticker.available {
            return false;
        }
        if Some(guild_id) == self.local_guild() {
            return true;
        }
        let premium = self.current_user.as_ref().is_some_and(|user| user.premium);
        let channel_allows = self.view == View::DirectMessages
            || self
                .selected_channel_info()
                .is_some_and(|channel| channel.can_use_external_stickers);
        premium && channel_allows
    }

    fn sticker_sections(&self) -> Vec<PickerSection> {
        let guilds: Vec<PickerSection> = self
            .stickers_by_guild()
            .filter_map(|(guild_id, stickers)| {
                let stickers: Vec<Sticker> = stickers
                    .iter()
                    .filter(|sticker| self.can_send_sticker(guild_id, sticker))
                    .cloned()
                    .collect();
                let guild = self.guilds.iter().find(|guild| guild.id == guild_id)?;
                (!stickers.is_empty()).then(|| PickerSection {
                    kind: SectionKind::Guild(guild_id),
                    title: guild.name.clone(),
                    icon: SectionIcon::Guild(guild.icon_url.clone()),
                    stickers,
                })
            })
            .collect();
        let packs = self.sticker_packs.packs.iter().filter_map(|pack| {
            Some(PickerSection {
                kind: SectionKind::Pack(pack.id),
                title: pack.name.clone(),
                icon: SectionIcon::Sticker(pack.cover.clone()?),
                stickers: pack.stickers.clone(),
            })
        });
        let mut sections: Vec<PickerSection> = guilds.into_iter().chain(packs).collect();

        // Only what can still be sent here, so a sticker from a guild left behind drops out.
        let recent: Vec<Sticker> = prefs::load()
            .recent_stickers
            .iter()
            .filter_map(|id| {
                sections
                    .iter()
                    .flat_map(|section| &section.stickers)
                    .find(|sticker| sticker.item.id.get() == *id)
                    .cloned()
            })
            .take(RECENT_LIMIT)
            .collect();
        sections.insert(
            0,
            PickerSection {
                kind: SectionKind::Recent,
                title: "Recently Used".into(),
                icon: SectionIcon::Recent,
                stickers: recent,
            },
        );
        sections
    }
}

/// The open picker keeps its order; the next one shows the change.
fn remember_sticker(id: u64) {
    prefs::update(|prefs| {
        let recent = &mut prefs.recent_stickers;
        recent.retain(|other| *other != id);
        recent.insert(0, id);
        recent.truncate(RECENT_LIMIT);
    });
}
