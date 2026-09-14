//! Joining, leaving, and toggling a call.
//!
//! Interface only for now — see [`voice`](crate::screens::home::voice). Every
//! method here moves local state and notifies; none of them reach the network.

use std::time::Duration;

use gpui::*;
use twilight_model::id::{Id, marker::ChannelMarker};

use crate::screens::home::HomeScreen;
use crate::screens::home::voice::{VoiceCall, VoiceKind, VoiceParticipant, VoiceStatus};

/// How long a join sits in [`VoiceStatus::Connecting`] before it reads as
/// connected. A real handshake replaces this.
const CONNECT_DELAY_MS: u64 = 600;

impl HomeScreen {
    /// Whether the active call is in this channel.
    pub(in crate::screens::home) fn in_voice_channel(&self, channel_id: Id<ChannelMarker>) -> bool {
        self.voice
            .as_ref()
            .is_some_and(|call| call.channel_id == channel_id)
    }

    /// Joins a guild voice channel, or leaves it if it's the one already
    /// joined — clicking the connected channel again toggles, like Discord.
    pub(in crate::screens::home) fn join_voice_channel(
        &mut self,
        channel_id: Id<ChannelMarker>,
        cx: &mut Context<Self>,
    ) {
        if self.in_voice_channel(channel_id) {
            self.leave_voice(cx);
            return;
        }

        let Some(channel) = self
            .channel_groups
            .iter()
            .flat_map(|group| &group.channels)
            .find(|channel| channel.id == channel_id)
        else {
            return;
        };
        let name = channel.name.clone();
        let guild_name = self
            .selected_guild
            .and_then(|id| self.guilds.iter().find(|guild| guild.id == id))
            .map(|guild| guild.name.clone());

        self.voice = Some(VoiceCall {
            channel_id,
            kind: VoiceKind::Channel,
            name,
            context: guild_name,
            status: VoiceStatus::Connecting,
            camera: false,
            screen_share: false,
            participants: vec![self.self_participant()],
        });
        // Opening the channel puts its stage in the content pane.
        self.selected_channel = Some(channel_id);
        cx.notify();

        // Stands in for the voice handshake: the panel reads "Connecting…"
        // for a beat before settling, so both states are on screen.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(CONNECT_DELAY_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(call) = &mut this.voice
                    && call.channel_id == channel_id
                {
                    call.status = VoiceStatus::Connected;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Places a call in the open DM conversation. The other side is listed as
    /// pending until it answers.
    pub(in crate::screens::home) fn start_dm_call(&mut self, camera: bool, cx: &mut Context<Self>) {
        let Some(dm) = self.selected_dm_info().cloned() else {
            return;
        };
        if self.in_voice_channel(dm.id) {
            return;
        }

        self.voice = Some(VoiceCall {
            channel_id: dm.id,
            kind: VoiceKind::Direct,
            name: dm.name.clone(),
            context: None,
            status: VoiceStatus::Ringing,
            camera,
            screen_share: false,
            participants: vec![
                self.self_participant(),
                VoiceParticipant::new(dm.name, dm.avatar_url).pending(),
            ],
        });
        cx.notify();
    }

    pub(in crate::screens::home) fn leave_voice(&mut self, cx: &mut Context<Self>) {
        self.voice = None;
        cx.notify();
    }

    /// Toggles self-mute. Unmuting while deafened undeafens too, since a
    /// deafened user can't be heard by anyone either way.
    pub(in crate::screens::home) fn toggle_voice_mute(&mut self, cx: &mut Context<Self>) {
        self.voice_muted = !self.voice_muted;
        if !self.voice_muted {
            self.voice_deafened = false;
        }
        cx.notify();
    }

    /// Toggles self-deafen, which implies mute while it's on.
    pub(in crate::screens::home) fn toggle_voice_deafen(&mut self, cx: &mut Context<Self>) {
        self.voice_deafened = !self.voice_deafened;
        if self.voice_deafened {
            self.voice_muted = true;
        }
        cx.notify();
    }

    pub(in crate::screens::home) fn toggle_voice_camera(&mut self, cx: &mut Context<Self>) {
        if let Some(call) = &mut self.voice {
            call.camera = !call.camera;
            cx.notify();
        }
    }

    pub(in crate::screens::home) fn toggle_screen_share(&mut self, cx: &mut Context<Self>) {
        if let Some(call) = &mut self.voice {
            call.screen_share = !call.screen_share;
            cx.notify();
        }
    }

    /// The signed-in user as a call participant, falling back to a neutral
    /// label while `GET /users/@me` is still in flight.
    fn self_participant(&self) -> VoiceParticipant {
        match &self.current_user {
            Some(user) => VoiceParticipant::new(user.name.clone(), user.avatar_url.clone())
                .user_id(user.id)
                .this_user(),
            None => VoiceParticipant::new("You".into(), None).this_user(),
        }
    }
}
