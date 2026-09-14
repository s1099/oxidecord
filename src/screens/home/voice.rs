//! The state behind a voice call, and the small types it holds.
//!
//! This is the interface layer only: nothing here speaks to Discord's voice
//! gateway or touches an audio device. Joining a channel fills a [`VoiceCall`]
//! in locally so the call chrome — the sidebar panel, the stage, the controls —
//! can be built and looked at ahead of the transport underneath it.

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, UserMarker},
};

/// Where a call is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceStatus {
    /// Handshaking with the voice server.
    Connecting,
    /// In the channel, audio flowing.
    Connected,
    /// An outgoing DM call nobody has picked up yet.
    Ringing,
}

impl VoiceStatus {
    /// The line shown under the channel name in the sidebar panel.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Connected => "Voice Connected",
            Self::Ringing => "Ringing…",
        }
    }
}

/// Whether the call sits in a guild's voice channel or in a DM conversation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceKind {
    Channel,
    Direct,
}

/// Someone in the call. The signed-in user is one of these too, flagged with
/// `is_self` — their mute and deafen live on the screen rather than here, since
/// both outlive any single call.
pub(super) struct VoiceParticipant {
    pub user_id: Option<Id<UserMarker>>,
    pub name: String,
    pub avatar_url: Option<String>,
    pub is_self: bool,
    /// Invited but not yet in the call, so the tile reads as waiting.
    pub pending: bool,
    pub muted: bool,
    pub deafened: bool,
}

impl VoiceParticipant {
    pub(super) fn new(name: String, avatar_url: Option<String>) -> Self {
        Self {
            user_id: None,
            name,
            avatar_url,
            is_self: false,
            pending: false,
            muted: false,
            deafened: false,
        }
    }

    pub(super) fn user_id(mut self, id: Id<UserMarker>) -> Self {
        self.user_id = Some(id);
        self
    }

    pub(super) fn this_user(mut self) -> Self {
        self.is_self = true;
        self
    }

    pub(super) fn pending(mut self) -> Self {
        self.pending = true;
        self
    }
}

/// The call the user is currently in. At most one at a time, like Discord.
pub(super) struct VoiceCall {
    /// The voice channel, or the DM the call is placed in.
    pub channel_id: Id<ChannelMarker>,
    pub kind: VoiceKind,
    /// The channel or conversation name, shown wherever the call is labelled.
    pub name: String,
    /// The guild the channel belongs to; `None` for a DM call.
    pub context: Option<String>,
    pub status: VoiceStatus,
    pub camera: bool,
    pub screen_share: bool,
    pub participants: Vec<VoiceParticipant>,
}
