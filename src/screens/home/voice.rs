//! The state behind a voice call, and the small types it holds.
//!
//! The call itself lives in [`crate::voice`]; this is what the screen needs to
//! draw one. Who is in the channel isn't stored here — that comes from the
//! voice states the gateway keeps up to date, so a participant list is always
//! built from the latest ones.

use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

/// Where a call is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceStatus {
    /// Waiting on the gateway for the voice server, then on the handshake with
    /// it.
    Connecting,
    /// Connected to the voice server, audio flowing.
    Connected,
}

impl VoiceStatus {
    /// The line shown under the channel name in the sidebar panel.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Connected => "Voice Connected",
        }
    }
}

/// The call the user is in. At most one at a time, like Discord.
pub(super) struct VoiceCall {
    /// The voice channel, or the DM the call is placed in.
    pub channel_id: Id<ChannelMarker>,
    /// The channel's guild. `None` for a DM call, which Discord treats as a
    /// guildless one.
    pub guild_id: Option<Id<GuildMarker>>,
    /// The channel or conversation name, shown wherever the call is labelled.
    pub name: String,
    /// The guild the channel belongs to; `None` for a DM call.
    pub context: Option<String>,
    pub status: VoiceStatus,
    /// Why the call failed, when it did.
    pub error: Option<String>,
}

/// The connection parameters, as the two gateway dispatches that answer a join
/// deliver them.
///
/// `VOICE_STATE_UPDATE` carries the session and `VOICE_SERVER_UPDATE` the
/// server, in either order, and the connection can only open once both have
/// landed.
#[derive(Default)]
pub(super) struct PendingVoice {
    pub session_id: Option<String>,
    pub token: Option<String>,
    pub endpoint: Option<String>,
}

/// Someone in a voice channel, assembled for display from their voice state.
pub(super) struct VoiceParticipant {
    pub user_id: Id<UserMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// Silenced, whether by themselves or by a moderator — the tile shows the
    /// badge either way.
    pub muted: bool,
    pub deafened: bool,
    /// Transmitting right now.
    pub speaking: bool,
    pub is_self: bool,
}
