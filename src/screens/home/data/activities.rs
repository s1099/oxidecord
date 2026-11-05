//! Starting, joining and leaving activities in the call, and keeping track of
//! the ones running anywhere.
//!
//! Launching is a request, but being in is the gateway's to say: Discord adds
//! the session to the instance's participants in `EMBEDDED_ACTIVITY_UPDATE_V2`,
//! and only then does the activity's window open, as in Discord's client.

use std::sync::Arc;

use gpui_kit::*;
use twilight_model::guild::Permissions;
use twilight_model::id::{
    Id,
    marker::{ApplicationMarker, ChannelMarker, GuildMarker},
};

use crate::discord::{self, ActivityApp, ActivityInstance};
use crate::screens::activity::{self, ActivityContext};
use crate::screens::home::HomeScreen;
use crate::screens::home::activities::{ActivitySession, ShelfState};
use crate::screens::home::commands::IndexState;
use crate::screens::home::voice::VoiceStatus;

use super::messages::next_nonce;

impl HomeScreen {
    pub(in crate::screens::home) fn ensure_activity_shelf(
        &mut self,
        guild_id: Option<Id<GuildMarker>>,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.activity_shelf.get(&guild_id),
            Some(ShelfState::Loading | ShelfState::Loaded(_))
        ) {
            return;
        }
        self.activity_shelf.insert(guild_id, ShelfState::Loading);
        cx.spawn(async move |this, cx| {
            let shelf = discord::fetch_activity_shelf(guild_id).await;
            let _ = this.update(cx, |this, cx| {
                let state = match shelf {
                    Ok(apps) => ShelfState::Loaded(Arc::new(apps)),
                    Err(err) => {
                        eprintln!("failed to fetch the activity shelf: {err}");
                        ShelfState::Failed
                    }
                };
                this.activity_shelf.insert(guild_id, state);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::screens::home) fn running_activities(
        &self,
        channel_id: Id<ChannelMarker>,
    ) -> &[ActivityInstance] {
        self.activity_instances
            .get(&channel_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Whether the user may start or join activities in the current call.
    /// Channel overwrites aren't tracked, so a guild's own permissions decide.
    pub(in crate::screens::home) fn can_use_activities(&self) -> bool {
        let Some(call) = &self.voice else {
            return false;
        };
        if call.status != VoiceStatus::Connected {
            return false;
        }
        match call.guild_id {
            Some(guild_id) => self.guild_permissions(guild_id).is_some_and(|permissions| {
                permissions.contains(Permissions::USE_EMBEDDED_ACTIVITIES)
            }),
            None => true,
        }
    }

    fn guild_permissions(&self, guild_id: Id<GuildMarker>) -> Option<Permissions> {
        let guild = self.guilds.iter().find(|guild| guild.id == guild_id)?;
        Some(
            if guild.owner || guild.permissions.contains(Permissions::ADMINISTRATOR) {
                Permissions::all()
            } else {
                guild.permissions
            },
        )
    }

    /// The shelf's entry for an app, when it's on the call's shelf.
    pub(in crate::screens::home) fn activity_app(
        &self,
        guild_id: Option<Id<GuildMarker>>,
        app_id: Id<ApplicationMarker>,
    ) -> Option<&ActivityApp> {
        match self.activity_shelf.get(&guild_id) {
            Some(ShelfState::Loaded(apps)) => apps.iter().find(|app| app.id == app_id),
            _ => None,
        }
    }

    /// Joins the app's instance in the call, or starts one if none is running.
    pub(in crate::screens::home) fn launch_activity(
        &mut self,
        app: ActivityApp,
        cx: &mut Context<Self>,
    ) {
        let Some(call) = &self.voice else {
            return;
        };
        let (channel_id, guild_id) = (call.channel_id, call.guild_id);
        let Some(session_id) = self.gateway_session.clone() else {
            return;
        };
        if self
            .activity
            .as_ref()
            .is_some_and(|session| session.app.id == app.id && session.channel_id == channel_id)
        {
            self.show_activity(cx);
            return;
        }
        self.end_activity(cx);
        self.activity_error = None;

        // Discord's client runs the entry point command only to start one.
        let joining = self
            .running_activities(channel_id)
            .iter()
            .any(|instance| instance.app_id == app.id);
        let nonce = (!joining).then(|| next_nonce().0);
        let id = self.next_activity_session;
        self.next_activity_session += 1;
        self.activity = Some(ActivitySession {
            id,
            app: app.clone(),
            channel_id,
            guild_id,
            in_call: true,
            nonce,
            instance: None,
            proxy_ticket: None,
            window: None,
        });
        cx.notify();

        let app_id = app.id;
        cx.spawn(async move |this, cx| {
            let ticket = discord::fetch_proxy_ticket(app_id, channel_id).await;
            let launched = match nonce {
                None => discord::join_activity(app_id, channel_id, guild_id, session_id).await,
                Some(nonce) => match discord::fetch_entry_point(app_id).await {
                    Ok(entry_point) => {
                        discord::start_activity(discord::ActivityLaunch {
                            app_id,
                            channel_id,
                            guild_id,
                            session_id,
                            nonce,
                            entry_point,
                        })
                        .await
                    }
                    Err(err) => Err(err),
                },
            };
            let _ = this.update(cx, |this, cx| {
                if this
                    .activity
                    .as_ref()
                    .is_none_or(|session| session.id != id)
                {
                    return;
                }
                match ticket.and_then(|ticket| launched.map(|()| ticket)) {
                    Ok(ticket) => {
                        if let Some(session) = &mut this.activity {
                            session.proxy_ticket = Some(ticket);
                        }
                        // The instance can be reported before the request returns.
                        this.open_activity_if_ready(cx);
                    }
                    Err(err) => {
                        this.fail_activity(format!("Couldn't start {}: {err}", app.name), cx)
                    }
                }
            });
        })
        .detach();
    }

    fn fail_activity(&mut self, error: String, cx: &mut Context<Self>) {
        // Only a call's panel shows it, and a text channel's activity has none.
        eprintln!("{error}");
        self.end_activity(cx);
        self.activity_error = Some(error);
        cx.notify();
    }

    /// The app refused the launch interaction, or never answered it.
    pub(in crate::screens::home) fn activity_launch_failed(
        &mut self,
        nonce: u64,
        timed_out: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .activity
            .as_ref()
            .filter(|session| session.nonce == Some(nonce))
        else {
            return;
        };
        let name = session.app.name.clone();
        let error = if timed_out {
            format!("{name} didn't respond")
        } else {
            format!("Couldn't start {name}")
        };
        self.fail_activity(error, cx);
    }

    pub(in crate::screens::home) fn handle_activity_instance(
        &mut self,
        instance: ActivityInstance,
        cx: &mut Context<Self>,
    ) {
        let instances = self
            .activity_instances
            .entry(instance.channel_id)
            .or_default();
        instances.retain(|known| known.composite_id != instance.composite_id);
        if !instance.participants.is_empty() {
            instances.push(instance.clone());
        }
        if instances.is_empty() {
            self.activity_instances.remove(&instance.channel_id);
        }

        let (Some(self_id), Some(session_id)) = (self.self_user_id, self.gateway_session.clone())
        else {
            return cx.notify();
        };
        let Some(session) = self.activity.as_mut().filter(|session| {
            session.app.id == instance.app_id && session.channel_id == instance.channel_id
        }) else {
            // Started some other way, by an app's command or another client of
            // this session: Discord's client opens whatever lists its session.
            if instance.includes(self_id, &session_id) {
                self.adopt_activity(instance, cx);
            }
            return cx.notify();
        };
        let ours = session
            .instance
            .as_ref()
            .is_some_and(|known| known.composite_id == instance.composite_id);
        if instance.includes(self_id, &session_id) {
            if let Some((_, window)) = &session.window {
                let update = instance.clone();
                let _ = window.update(cx, |window, cx| window.update_instance(update, cx));
            }
            session.instance = Some(instance);
            self.open_activity_if_ready(cx);
        } else if ours {
            // Removed from it, or it ended.
            self.end_activity(cx);
        }
        cx.notify();
    }

    /// Opens an instance the user was put into without launching it from here.
    fn adopt_activity(&mut self, instance: ActivityInstance, cx: &mut Context<Self>) {
        self.end_activity(cx);
        self.activity_error = None;
        let app = self
            .known_activity_app(instance.guild_id, instance.app_id)
            .unwrap_or_else(|| ActivityApp {
                id: instance.app_id,
                name: "Activity".into(),
                icon_url: None,
            });
        let id = self.next_activity_session;
        self.next_activity_session += 1;
        let (app_id, channel_id) = (instance.app_id, instance.channel_id);
        self.activity = Some(ActivitySession {
            id,
            app,
            channel_id,
            guild_id: instance.guild_id,
            in_call: self.in_voice_channel(channel_id),
            nonce: None,
            instance: Some(instance),
            proxy_ticket: None,
            window: None,
        });
        cx.spawn(async move |this, cx| {
            let ticket = discord::fetch_proxy_ticket(app_id, channel_id).await;
            let _ = this.update(cx, |this, cx| {
                let Some(session) = this.activity.as_mut().filter(|session| session.id == id)
                else {
                    return;
                };
                match ticket {
                    Ok(ticket) => {
                        session.proxy_ticket = Some(ticket);
                        this.open_activity_if_ready(cx);
                    }
                    Err(err) => {
                        let error = format!("Couldn't open {}: {err}", session.app.name);
                        this.fail_activity(error, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// An app's name and icon from the shelf, or from a command index that
    /// lists it, which covers an activity started by a slash command.
    fn known_activity_app(
        &self,
        guild_id: Option<Id<GuildMarker>>,
        app_id: Id<ApplicationMarker>,
    ) -> Option<ActivityApp> {
        if let Some(app) = self.activity_app(guild_id, app_id) {
            return Some(app.clone());
        }
        self.command_indexes.values().find_map(|index| {
            let IndexState::Loaded(index) = index else {
                return None;
            };
            let app = index
                .commands
                .iter()
                .filter_map(|command| command.app())
                .find(|app| app.id == app_id)?;
            Some(ActivityApp {
                id: app.id,
                name: app.name.clone(),
                icon_url: app.icon_url.clone(),
            })
        })
    }

    pub(in crate::screens::home) fn handle_guild_activities(
        &mut self,
        guild_id: Id<GuildMarker>,
        instances: Vec<ActivityInstance>,
        cx: &mut Context<Self>,
    ) {
        self.activity_instances.retain(|_, running| {
            running.retain(|instance| instance.guild_id != Some(guild_id));
            !running.is_empty()
        });
        for instance in instances {
            if !instance.participants.is_empty() {
                self.activity_instances
                    .entry(instance.channel_id)
                    .or_default()
                    .push(instance);
            }
        }
        cx.notify();
    }

    fn open_activity_if_ready(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.activity else {
            return;
        };
        let (Some(instance), Some(proxy_ticket), None, Some(self_id)) = (
            session.instance.clone(),
            session.proxy_ticket.clone(),
            &session.window,
            self.self_user_id,
        ) else {
            return;
        };
        let call_channel = self
            .voice
            .as_ref()
            .filter(|call| call.channel_id == session.channel_id);
        let guild_channel = self.channel_info(session.channel_id);
        let channel_name = call_channel
            .map(|call| call.name.clone())
            .or_else(|| guild_channel.map(|channel| channel.name.clone()))
            .unwrap_or_default();
        let text_channel = guild_channel.is_some_and(|channel| !channel.kind.is_voice());
        let group_dm = session.guild_id.is_none()
            && self
                .dms
                .list
                .iter()
                .any(|dm| dm.id == session.channel_id && dm.recipient_id.is_none());
        let context = ActivityContext {
            app: session.app.clone(),
            instance,
            channel_name,
            channel_kind: match (session.guild_id, group_dm) {
                (Some(_), _) if text_channel => 0,
                (Some(_), _) => 2,
                (None, true) => 3,
                (None, false) => 1,
            },
            permissions: match session.guild_id {
                Some(guild_id) => self
                    .guild_permissions(guild_id)
                    .unwrap_or(Permissions::empty()),
                None => dm_permissions(),
            },
            proxy_ticket,
            self_id,
        };
        let id = session.id;
        let screen = cx.entity().downgrade();
        let window = activity::open_activity_window(
            context,
            move |cx| {
                let _ = screen.update(cx, |this, cx| this.activity_window_closed(id, cx));
            },
            cx,
        );
        match window {
            Some(window) => {
                if let Some(session) = &mut self.activity {
                    session.window = Some(window);
                }
            }
            None => self.fail_activity("Couldn't open the activity's window".into(), cx),
        }
    }

    fn activity_window_closed(&mut self, id: u64, cx: &mut Context<Self>) {
        if self
            .activity
            .as_ref()
            .is_some_and(|session| session.id == id)
        {
            self.end_activity(cx);
        }
    }

    /// Brings the open activity's window to the front.
    pub(in crate::screens::home) fn show_activity(&mut self, cx: &mut Context<Self>) {
        if let Some((handle, _)) = self.activity.as_ref().and_then(|s| s.window.as_ref()) {
            let _ = cx.update_window(*handle, |_, window, _| window.activate_window());
        }
    }

    /// Leaves the activity, closing its window if it's open.
    pub(in crate::screens::home) fn end_activity(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.activity.take() else {
            return;
        };
        if let Some((handle, _)) = session.window {
            // Deferred: the window may be the one reporting its own close.
            cx.defer(move |cx| activity::close_activity_window(handle, cx));
        }
        if let (Some(instance), Some(session_id)) = (session.instance, self.gateway_session.clone())
        {
            cx.spawn(async move |_, _| {
                if let Err(err) = discord::leave_activity(
                    instance.app_id,
                    instance.location_id,
                    instance.instance_id,
                    session_id,
                )
                .await
                {
                    eprintln!("failed to leave the activity: {err}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// An activity in a call belongs to it; leaving or being moved out of that
    /// call ends it.
    pub(in crate::screens::home) fn end_activity_outside_call(&mut self, cx: &mut Context<Self>) {
        let stranded = self
            .activity
            .as_ref()
            .is_some_and(|session| session.in_call && !self.in_voice_channel(session.channel_id));
        if stranded {
            self.end_activity(cx);
        }
    }
}

/// Outside a guild nothing is denied; these are what a call can use.
fn dm_permissions() -> Permissions {
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::STREAM
        | Permissions::USE_VAD
        | Permissions::USE_EMBEDDED_ACTIVITIES
}
