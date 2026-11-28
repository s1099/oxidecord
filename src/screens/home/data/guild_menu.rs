//! What the right-click menus on servers, folders and channels do: marking
//! them read and leaving a server.

use gpui_kit::component::{WindowExt as _, button::ButtonVariant, dialog::DialogButtonProps};
use gpui_kit::*;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker},
};

use crate::discord;
use crate::screens::home::{HomeScreen, View};
use crate::ui::{depth, dialogs};

impl HomeScreen {
    /// Every channel and joined thread in them, in one bulk ack.
    pub(in crate::screens::home) fn mark_guilds_read(
        &mut self,
        guild_ids: &[Id<GuildMarker>],
        cx: &mut Context<Self>,
    ) {
        let acks: Vec<_> = guild_ids
            .iter()
            .flat_map(|&guild_id| self.unreads.guild_unacked(guild_id))
            .collect();
        let clears_open = self.view == View::Guild
            && self
                .selected_guild
                .is_some_and(|id| guild_ids.contains(&id));
        self.ack_now(acks, clears_open, cx);
    }

    /// The open server, as Discord's shift+escape does.
    pub(in crate::screens::home) fn mark_open_guild_read(&mut self, cx: &mut Context<Self>) {
        if self.view != View::Guild {
            return;
        }
        if let Some(guild_id) = self.selected_guild {
            self.mark_guilds_read(&[guild_id], cx);
        }
    }

    pub(in crate::screens::home) fn mark_channel_read(
        &mut self,
        channel_id: Id<ChannelMarker>,
        cx: &mut Context<Self>,
    ) {
        if !self.unreads.is_unread(channel_id) && self.unreads.mentions(channel_id) == 0 {
            return;
        }
        let acks: Vec<_> = self
            .unreads
            .latest(channel_id)
            .map(|latest| (channel_id, latest))
            .into_iter()
            .collect();
        let clears_open = self.selected_channel == Some(channel_id);
        self.ack_now(acks, clears_open, cx);
    }

    /// Skips the ack batching, since the user asked for it outright. `clears_open` drops the open
    /// conversation's "new" divider and any mark-unread hold along with it.
    fn ack_now(
        &mut self,
        acks: Vec<(Id<ChannelMarker>, Id<MessageMarker>)>,
        clears_open: bool,
        cx: &mut Context<Self>,
    ) {
        if acks.is_empty() {
            return;
        }
        for &(channel_id, message_id) in &acks {
            self.unreads.ack(channel_id, message_id);
            self.pending_acks.remove(&channel_id);
        }
        if clears_open {
            self.marked_unread = false;
            self.set_unread_marker(None);
        }
        cx.notify();
        cx.spawn(async move |_, _| {
            if let Err(err) = discord::ack_bulk(acks).await {
                log::error!("marking read failed: {err}");
            }
        })
        .detach();
    }

    pub(in crate::screens::home) fn confirm_leave_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)
            .map(|guild| guild.name.clone())
        else {
            return;
        };
        let screen = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let screen = screen.clone();
            dialog
                .confirm()
                .border_color(depth::ring(cx))
                .width(px(440.))
                .title(SharedString::from(format!("Leave '{name}'")))
                .child(format!(
                    "Are you sure you want to leave {name}? You won't be able to rejoin this \
                     server unless you are re-invited."
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Leave Server")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    if let Some(screen) = screen.upgrade() {
                        screen.update(cx, |this, cx| this.leave_guild(guild_id, window, cx));
                    }
                    true
                })
        });
    }

    fn leave_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = discord::leave_guild(guild_id).await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                // The gateway's `GUILD_DELETE` would too, but may lag.
                Ok(()) => this.remove_guild(guild_id, window, cx),
                Err(err) => dialogs::show_error("Couldn't leave the server", [err], window, cx),
            });
        })
        .detach();
    }

    /// Gone from the user's list: left, kicked, or deleted.
    pub(in crate::screens::home) fn remove_guild(
        &mut self,
        guild_id: Id<GuildMarker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.guilds.len();
        self.guilds.retain(|guild| guild.id != guild_id);
        if self.guilds.len() == before {
            return;
        }
        self.rebuild_rail();
        if self
            .voice
            .as_ref()
            .is_some_and(|call| call.guild_id == Some(guild_id))
        {
            self.leave_voice(cx);
        }
        if self.selected_guild == Some(guild_id) {
            let in_guild = self.view == View::Guild;
            // Cleared first, so the collapsed categories aren't saved for a guild that's gone.
            self.selected_guild = None;
            if in_guild {
                self.open_direct_messages(window, cx);
            }
        }
        cx.notify();
    }
}
