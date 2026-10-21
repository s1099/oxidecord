//! Loading the signed-in user, their guilds, and a guild's channels, and
//! logging them out.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, Channel};
use crate::platform::prefs;
use crate::screens::home::channels::build_channel_groups;
use crate::screens::home::folders::build_rail_entries;
use crate::screens::home::{HomeScreen, SessionEnded, View};

impl HomeScreen {
    /// Logs out and hands back to login. A call is left properly first, so
    /// Discord doesn't keep showing the user in the channel until the dropped
    /// connection times out.
    pub(in crate::screens::home) fn log_out(&mut self, cx: &mut Context<Self>) {
        if self.voice.is_some() {
            self.leave_voice(cx);
        }
        discord::log_out();
        cx.emit(SessionEnded);
    }

    pub(in crate::screens::home) fn load_guilds(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = discord::fetch_guilds().await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        this.rebuild_rail();
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, window, cx);
                        }
                    }
                    // A rejected token is forgotten by the request itself.
                    Err(_) if discord::load_token().is_none() => {
                        cx.emit(SessionEnded);
                        return;
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Loads the user's settings: the folders that decide the rail's order,
    /// and their chosen status. Best-effort: until it lands the rail uses the
    /// plain guild list.
    pub(in crate::screens::home) fn load_user_settings(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = discord::fetch_user_settings().await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(settings) => {
                        this.guild_folders = Some(settings.guild_folders);
                        // A status picked while this was in flight wins.
                        if this.presence_status.is_none() {
                            this.apply_presence_status(settings.status.unwrap_or_default());
                        }
                    }
                    Err(err) => {
                        eprintln!("failed to load user settings: {err}");
                        return;
                    }
                }
                this.rebuild_rail();
                cx.notify();
            });
        })
        .detach();
    }

    fn rebuild_rail(&mut self) {
        self.rail_entries = build_rail_entries(&self.guilds, self.guild_folders.as_ref());
    }

    /// Loads the signed-in user for the sidebar account panel. Best-effort:
    /// on failure the panel just stays empty.
    pub(in crate::screens::home) fn load_current_user(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Ok(user) = discord::fetch_current_user().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Whichever of this and the gateway's READY lands first fills
                // in the id a voice connection identifies itself with.
                this.self_user_id.get_or_insert(user.id);
                this.current_user = Some(user);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn select_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click on another guild cancels a jump still in flight.
        self.pending_message_target = None;
        self.open_guild_channel_at(guild_id, None, window, cx);
    }

    /// Opens `guild_id` and selects `target_channel` — or the guild's first
    /// text channel when `None` — as a jump's landing spot. When the guild's
    /// channels are still loading, the selection is held until they land.
    pub(in crate::screens::home) fn open_guild_channel_at(
        &mut self,
        guild_id: Id<GuildMarker>,
        target_channel: Option<Id<ChannelMarker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_guild = self.selected_guild == Some(guild_id);
        if self.view == View::Guild && same_guild {
            if self.channels_loading {
                self.pending_channel_selection = target_channel;
                return;
            }
            if let Some(channel_id) = target_channel.filter(|&id| self.is_selectable_channel(id)) {
                self.apply_channel_selection(channel_id, window, cx);
            }
            return;
        }
        self.save_collapsed_categories();
        self.view = View::Guild;
        // Returning from the DM view to the guild that's already loaded:
        // switch back to its channels, reopening the jumped-to one instead of
        // the channel that was open, since the DM view cleared the selection.
        if same_guild && !self.channel_groups.is_empty() {
            let chosen = target_channel
                .filter(|&id| self.is_selectable_channel(id))
                .or_else(|| self.first_text_channel());
            if let Some(channel_id) = chosen {
                self.apply_channel_selection(channel_id, window, cx);
            }
            cx.notify();
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_groups.clear();
        self.selected_channel = None;
        self.load_collapsed_categories();
        self.channels_error = None;
        self.channels_loading = false;

        match self.guild_channels.get(&guild_id).cloned() {
            Some(channels) => {
                self.channel_groups = build_channel_groups(channels);
                let chosen = target_channel
                    .filter(|&id| self.is_selectable_channel(id))
                    .or_else(|| self.first_text_channel());
                if let Some(channel_id) = chosen {
                    self.apply_channel_selection(channel_id, window, cx);
                }
                if self.stale_guild_channels.contains(&guild_id) {
                    self.fetch_guild_channels(guild_id, window, cx);
                }
            }
            // Only until the gateway's `READY` lands, or for a guild it sent
            // as unavailable.
            None => {
                self.channels_loading = true;
                self.pending_channel_selection = target_channel;
                self.fetch_guild_channels(guild_id, window, cx);
            }
        }
        cx.notify();
    }

    /// Whether a channel can be selected in the sidebar: it is in the open
    /// guild's list and isn't a voice channel.
    fn is_selectable_channel(&self, channel_id: Id<ChannelMarker>) -> bool {
        self.channel_info(channel_id)
            .is_some_and(|channel| !channel.kind.is_voice())
    }

    /// Fetches a guild's channels over REST: when the gateway hasn't sent
    /// them yet, or the list it sent has since changed. A fetch for the open
    /// guild that has a list on screen already swaps the new one in under it.
    fn fetch_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.fetching_guild_channels.insert(guild_id) {
            return;
        }
        // The guild list already carries the user's guild-wide permissions, so
        // channel visibility only needs the member object on top of them.
        let (base_permissions, owner) = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| (guild.permissions, guild.owner))
            .unwrap_or((discord::Permissions::empty(), false));

        cx.spawn_in(window, async move |this, cx| {
            let result = discord::fetch_channels(guild_id, base_permissions, owner).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching_guild_channels.remove(&guild_id);
                match result {
                    Ok(channels) => {
                        for channel in &channels {
                            if let Some(latest) = channel.last_message_id {
                                this.unreads.note_latest(channel.id, latest);
                            }
                        }
                        this.set_guild_channels(guild_id, channels, window, cx);
                    }
                    // A failed refetch leaves the list that's on screen.
                    Err(err) if this.channels_loading && this.selected_guild == Some(guild_id) => {
                        this.channels_error = Some(err);
                        this.channels_loading = false;
                        cx.notify();
                    }
                    Err(err) => eprintln!("refreshing a guild's channels failed: {err}"),
                }
            });
        })
        .detach();
    }

    /// Takes a guild's channels as they now stand, from the gateway or a
    /// fetch, and redraws the sidebar if it's the open guild's.
    pub(in crate::screens::home) fn set_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        channels: Vec<Channel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stale_guild_channels.remove(&guild_id);
        let open = self.view == View::Guild && self.selected_guild == Some(guild_id);
        if open {
            self.channel_groups = build_channel_groups(channels.clone());
        }
        self.guild_channels.insert(guild_id, channels);
        if !open {
            return;
        }
        // The list was loading with nothing open, or the open channel is
        // gone from it. A selection requested while loading — a jump's —
        // wins over the remembered one.
        if self.channels_loading
            || self
                .selected_channel
                .is_none_or(|id| self.channel_info(id).is_none())
        {
            self.channels_loading = false;
            self.channels_error = None;
            self.selected_channel = None;
            match self
                .pending_channel_selection
                .take()
                .filter(|&id| self.is_selectable_channel(id))
            {
                Some(channel_id) => self.apply_channel_selection(channel_id, window, cx),
                None => self.open_guild_channel(window, cx),
            }
        }
        cx.notify();
    }

    /// Marks a guild's channel list out of date: a channel in it changed, or
    /// something deciding which the user can see. The open guild's is
    /// refetched now; any other's when it's next opened.
    pub(in crate::screens::home) fn invalidate_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guild_channels.contains_key(&guild_id) {
            return;
        }
        self.stale_guild_channels.insert(guild_id);
        if self.view == View::Guild && self.selected_guild == Some(guild_id) {
            self.fetch_guild_channels(guild_id, window, cx);
        }
    }

    /// Opens the channel last open in the selected guild, or its first text
    /// channel when that's gone or there wasn't one.
    fn open_guild_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last = self
            .selected_guild
            .and_then(|guild_id| self.last_guild_channel.get(&guild_id))
            .copied()
            .filter(|&id| {
                self.channel_info(id)
                    .is_some_and(|channel| !channel.kind.is_voice())
            });
        if let Some(channel_id) = last.or_else(|| self.first_text_channel()) {
            self.select_channel(channel_id, window, cx);
        }
    }

    fn load_collapsed_categories(&mut self) {
        let Some(guild_id) = self.selected_guild else {
            return;
        };
        self.collapsed_categories = prefs::load()
            .collapsed_categories
            .remove(&guild_id.get())
            .unwrap_or_default()
            .into_iter()
            .filter_map(Id::new_checked)
            .collect();
    }

    /// Writes the selected guild's collapsed categories to the prefs file.
    /// Called on channel and guild switches rather than on every toggle, so
    /// clicking through categories doesn't rewrite the file each time.
    pub(in crate::screens::home) fn save_collapsed_categories(&self) {
        let Some(guild_id) = self.selected_guild else {
            return;
        };
        let mut collapsed: Vec<u64> = self
            .collapsed_categories
            .iter()
            .map(|id| id.get())
            .collect();
        collapsed.sort_unstable();

        let mut prefs = prefs::load();
        let stored = prefs.collapsed_categories.get(&guild_id.get());
        if stored.map_or(collapsed.is_empty(), |stored| *stored == collapsed) {
            return;
        }
        if collapsed.is_empty() {
            prefs.collapsed_categories.remove(&guild_id.get());
        } else {
            prefs.collapsed_categories.insert(guild_id.get(), collapsed);
        }
        prefs::save(&prefs);
    }

    pub(in crate::screens::home) fn selected_channel_info(&self) -> Option<&Channel> {
        self.channel_info(self.selected_channel?)
    }

    pub(in crate::screens::home) fn channel_info(&self, id: Id<ChannelMarker>) -> Option<&Channel> {
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == id)
    }

    /// The first text channel in display order, used as the default selection
    /// when entering a guild.
    fn first_text_channel(&self) -> Option<Id<ChannelMarker>> {
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| !channel.kind.is_voice())
            .map(|channel| channel.id)
    }
}
