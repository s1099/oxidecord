//! Loading the signed-in user, their guilds, and a guild's channels.

use gpui::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker},
};

use crate::discord::{self, Channel};
use crate::screens::home::channels::build_channel_groups;
use crate::screens::home::{HomeScreen, View};

impl HomeScreen {
    pub(crate) fn load_guilds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            self.loading = false;
            self.error = Some("No token found. Please log in first.".into());
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_guilds(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(guilds) => {
                        let first = guilds.first().map(|guild| guild.id);
                        this.guilds = guilds;
                        this.error = None;
                        if let Some(guild_id) = first {
                            this.select_guild(guild_id, window, cx);
                        }
                    }
                    Err(err) => this.error = Some(err),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Loads the signed-in user for the sidebar account panel. Best-effort:
    /// on failure the panel just stays empty.
    pub(crate) fn load_current_user(&mut self, cx: &mut Context<Self>) {
        let Some(token) = discord::load_token() else {
            return;
        };

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_current_user(token, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let Ok(Ok(user)) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.current_user = Some(user);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn select_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_guild = self.selected_guild == Some(guild_id);
        if self.view == View::Guild && same_guild {
            return;
        }
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
        self.collapsed_categories.clear();
        self.channels_error = None;
        self.channels_loading = true;
        cx.notify();

        let Some(token) = discord::load_token() else {
            self.channels_loading = false;
            self.channels_error = Some("No token found. Please log in first.".into());
            return;
        };

        // The guild list already carries the user's guild-wide permissions, so
        // channel visibility only needs the member object on top of them.
        let (base_permissions, owner) = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| (guild.permissions, guild.owner))
            .unwrap_or((discord::Permissions::empty(), false));

        let (tx, rx) = futures::channel::oneshot::channel();
        discord::fetch_channels(token, guild_id, base_permissions, owner, move |result| {
            let _ = tx.send(result);
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(result) = rx.await else {
                return;
            };

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

    pub(crate) fn selected_channel_info(&self) -> Option<&Channel> {
        let id = self.selected_channel?;
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == id)
    }

    /// The first text channel in display order used as the
    /// default selection when entering a guild
    fn first_text_channel(&self) -> Option<Id<ChannelMarker>> {
        self.channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| !channel.kind.is_voice())
            .map(|channel| channel.id)
    }
}
