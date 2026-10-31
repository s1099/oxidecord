//! The state behind a voice call, and the small types it holds.
//!
//! The call itself lives in [`crate::voice`]. Participants aren't stored here;
//! they're built from the gateway's latest voice states.

use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::RenderImage;

use crate::platform::capture::CaptureSource;
use crate::platform::h264::DecoderEvent;
use crate::voice::stream::{GoLive, WatchStream};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceStatus {
    /// Waiting on the gateway for the voice server, then on the handshake.
    Connecting,
    Connected,
}

impl VoiceStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting…",
            Self::Connected => "Voice Connected",
        }
    }
}

/// At most one at a time, like Discord.
pub(super) struct VoiceCall {
    pub channel_id: Id<ChannelMarker>,
    /// `None` for a DM call.
    pub guild_id: Option<Id<GuildMarker>>,
    pub name: String,
    /// Guild name; `None` for a DM call.
    pub context: Option<String>,
    pub status: VoiceStatus,
    pub error: Option<String>,
    /// A DM call is rung only after the user is connected: Discord has no call
    /// to ring until someone is in it.
    pub ring: bool,
    /// Kept after connecting because a stream opens under the same session.
    pub session_id: Option<String>,
    pub share_error: Option<String>,
    pub watch_error: Option<String>,
}

/// `STREAM_CREATE` carries the server id and `STREAM_SERVER_UPDATE` the
/// server, in either order, so the source waits here until both have landed.
pub(super) struct ScreenShare {
    pub stream_key: String,
    pub source_name: String,
    pub source: Option<CaptureSource>,
    pub server_id: Option<String>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    /// Dropping it stops the capture and the connection.
    pub stream: Option<GoLive>,
    pub live: bool,
}

/// Like [`ScreenShare`], waits for both `STREAM_CREATE` and `STREAM_SERVER_UPDATE`.
pub(super) struct StreamWatch {
    pub stream_key: String,
    pub streamer_id: Id<UserMarker>,
    pub server_id: Option<String>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    /// Cloned into each connection so a reopen after Discord moves the stream
    /// feeds the same pump.
    pub frames: UnboundedSender<DecoderEvent>,
    /// Dropping it closes the connection and stops the decoder.
    pub stream: Option<WatchStream>,
    /// Returned to gpui's sprite atlas when the next frame replaces it.
    pub frame: Option<Arc<RenderImage>>,
}

/// `VOICE_STATE_UPDATE` carries the session and `VOICE_SERVER_UPDATE` the
/// server, in either order; the connection opens once both have landed.
#[derive(Default)]
pub(super) struct PendingVoice {
    pub session_id: Option<String>,
    pub token: Option<String>,
    pub endpoint: Option<String>,
}

pub(super) struct VoiceParticipant {
    pub user_id: Id<UserMarker>,
    pub name: String,
    pub avatar_url: Option<String>,
    /// Banner colour as `0xRRGGBB`; only known once their profile has loaded.
    pub accent_color: Option<u32>,
    /// Self or server mute; the tile shows the badge either way.
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
    pub streaming: bool,
    pub is_self: bool,
}

/// Kept on the screen, not the call, so they survive between calls.
pub(super) struct VoiceSettings {
    /// Deafening silences the microphone too, so this can be on without the
    /// user pressing mute.
    pub muted: bool,
    pub deafened: bool,
    /// Restored on undeafen.
    pub mute_before_deafen: bool,
    /// `None` follows the system default. Persisted in prefs.
    pub input_device: Option<String>,
    /// `None` follows the system default. Persisted in prefs.
    pub output_device: Option<String>,
}
