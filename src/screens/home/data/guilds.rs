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
        let same_guild = self.selected_guild == Some(guild_id);
        if self.view == View::Guild && same_guild {
            return;
        }
        self.save_collapsed_categories();
        self.view = View::Guild;
        // Returning from the DM view to the guild that's already loaded: switch
        // back to its channels without refetching them, reopening a channel
        // since the DM view cleared the previous selection.
        if same_guild && !self.channel_groups.is_empty() {
            if let Some(first) = self.first_text_channel() {
                self.select_channel(first, window, cx);
            }
            cx.notify();
            return;
        }
        self.selected_guild = Some(guild_id);
        self.channel_groups.clear();
        self.selected_channel = None;
        self.load_collapsed_categories();
        self.channels_error = None;
        self.channels_loading = true;
        cx.notify();

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
                // The user may have clicked another guild while this request
                // was in flight; drop the stale response.
                if this.selected_guild != Some(guild_id) {
                    return;
                }
                match result {
                    Ok(channels) => {
                        this.channel_groups = build_channel_groups(channels);
                        if let Some(channel_id) = this.first_text_channel() {
                            this.select_channel(channel_id, window, cx);
                        }
                    }
                    Err(err) => this.channels_error = Some(err),
                }
                this.channels_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Restores the selected guild's collapsed categories from the prefs file.
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

    /// A channel of the loaded guild, by id.
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
