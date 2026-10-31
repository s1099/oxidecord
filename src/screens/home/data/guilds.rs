//! Loading the signed-in user, their guilds, and a guild's channels, and
//! logging them out.

use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, Channel};
use crate::platform::prefs;
use crate::screens::home::channels::{ChannelGroup, build_channel_groups};
use crate::screens::home::folders::build_rail_entries;
use crate::screens::home::{HomeScreen, SessionEnded, View};

impl HomeScreen {
    /// A call is left first so Discord doesn't keep showing the user in the channel
    /// until the dropped connection times out.
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

    /// Folders (the rail's order) and chosen status. Best-effort: until it lands the
    /// rail uses the plain guild list.
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

    /// Best-effort; on failure the panel stays empty.
    pub(in crate::screens::home) fn load_current_user(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Ok(user) = discord::fetch_current_user().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Whichever of this and READY lands first fills in the id voice identifies itself
                // with.
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
        let same_guild = self.selected_guild == Some(guild_id);
        if self.view == View::Guild && same_guild {
            return;
        }
        self.save_collapsed_categories();
        self.view = View::Guild;
        // Returning from the DM view, which cleared the selection: reopen the channel
        // that was open.
        if same_guild && !self.channel_groups.is_empty() {
            self.open_guild_channel(window, cx);
            cx.notify();
            return;
        }
        self.selected_guild = Some(guild_id);
        self.set_channel_groups(Vec::new());
        self.selected_channel = None;
        self.load_collapsed_categories();
        self.channels_error = None;
        self.channels_loading = false;

        match self.guild_data.channels.get(&guild_id).cloned() {
            Some(channels) => {
                self.set_channel_groups(build_channel_groups(channels));
                self.open_guild_channel(window, cx);
                if self.guild_data.stale_channels.contains(&guild_id) {
                    self.fetch_guild_channels(guild_id, window, cx);
                }
            }
            // Only until the gateway's `READY` lands, or for a guild it sent
            // as unavailable.
            None => {
                self.channels_loading = true;
                self.fetch_guild_channels(guild_id, window, cx);
            }
        }
        cx.notify();
    }

    /// For when the gateway hasn't sent a list or it went stale. The open guild's
    /// swaps in under the list on screen.
    fn fetch_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guild_data.fetching_channels.insert(guild_id) {
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
                this.guild_data.fetching_channels.remove(&guild_id);
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

    pub(in crate::screens::home) fn set_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        channels: Vec<Channel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.guild_data.stale_channels.remove(&guild_id);
        let open = self.view == View::Guild && self.selected_guild == Some(guild_id);
        if open {
            self.set_channel_groups(build_channel_groups(channels.clone()));
        }
        self.guild_data.channels.insert(guild_id, channels);
        if !open {
            return;
        }
        // The list was loading with nothing open, or the open channel is
        // gone from it.
        if self.channels_loading
            || self
                .selected_channel
                .is_none_or(|id| self.channel_info(id).is_none())
        {
            self.channels_loading = false;
            self.channels_error = None;
            self.selected_channel = None;
            self.open_guild_channel(window, cx);
        }
        cx.notify();
    }

    /// Marks a list stale: a channel changed, or something deciding visibility. The
    /// open guild's refetches now, any other's when next opened.
    pub(in crate::screens::home) fn invalidate_guild_channels(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.guild_data.channels.contains_key(&guild_id) {
            return;
        }
        self.guild_data.stale_channels.insert(guild_id);
        if self.view == View::Guild && self.selected_guild == Some(guild_id) {
            self.fetch_guild_channels(guild_id, window, cx);
        }
    }

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

    /// Called on channel and guild switches, not every toggle, so clicking through
    /// categories doesn't rewrite the file.
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
        let &(group, channel) = self.channel_index.get(&id)?;
        self.channel_groups.get(group)?.channels.get(channel)
    }

    fn set_channel_groups(&mut self, groups: Vec<ChannelGroup>) {
        self.channel_index = groups
            .iter()
            .enumerate()
            .flat_map(|(group_ix, group)| {
                group
                    .channels
                    .iter()
                    .enumerate()
                    .map(move |(channel_ix, channel)| (channel.id, (group_ix, channel_ix)))
            })
            .collect();
        self.channel_groups = groups;
    }

    fn first_text_channel(&self) -> Option<Id<ChannelMarker>> {
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| !channel.kind.is_voice())
            .map(|channel| channel.id)
    }
}
