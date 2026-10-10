//! The state behind a voice call, and the small types it holds.
//!
//! The call itself lives in [`crate::voice`]; this is what the screen needs to
//! draw one. Who is in the channel isn't stored here — that comes from the
//! voice states the gateway keeps up to date, so a participant list is always
//! built from the latest ones.

use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use gpui::RenderImage;

use crate::platform::capture::CaptureSource;
use crate::platform::h264::DecoderEvent;
use crate::voice::stream::{GoLive, WatchStream};
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
    /// Whether the DM's other members still need ringing. Set when the user
    /// places a DM call, and cleared once the user is connected and the ring goes out:
    /// Discord only has a call to ring once someone is in it.
    pub ring: bool,
    /// The voice session the gateway assigned the join. A stream is opened
    /// under the same session, so it's kept after the call connects.
    pub session_id: Option<String>,
    /// Why the last screen share couldn't start or stopped early.
    pub share_error: Option<String>,
    /// Why the last stream the user watched couldn't open or stopped early.
    pub watch_error: Option<String>,
}

/// The user's own stream into the call: what's being shared, and the
/// connection parameters as the gateway delivers them.
///
/// `STREAM_CREATE` carries the server id and `STREAM_SERVER_UPDATE` the
/// server, in either order, so the source waits here until both have landed.
pub(super) struct ScreenShare {
    /// Names the stream in every gateway command and dispatch about it.
    pub stream_key: String,
    /// The window title or display name, for the panel.
    pub source_name: String,
    /// The picked source, until the connection takes it.
    pub source: Option<CaptureSource>,
    pub server_id: Option<String>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    /// The running stream. Dropping it stops the capture and the connection.
    pub stream: Option<GoLive>,
    /// Frames are going out.
    pub live: bool,
}

/// Someone else's stream being watched: whose, the connection parameters as
/// the gateway delivers them, and the picture on screen.
///
/// Like [`ScreenShare`], the connection waits here until both `STREAM_CREATE`
/// and `STREAM_SERVER_UPDATE` have landed.
pub(super) struct StreamWatch {
    pub stream_key: String,
    pub streamer_id: Id<UserMarker>,
    pub server_id: Option<String>,
    pub endpoint: Option<String>,
    pub token: Option<String>,
    /// Where decoded pictures go. Cloned into each connection, so one opened
    /// again after Discord moves the stream feeds the same pump.
    pub frames: UnboundedSender<DecoderEvent>,
    /// The running connection. Dropping it closes it and stops the decoder.
    pub stream: Option<WatchStream>,
    /// The picture on screen, handed back to gpui's sprite atlas when the
    /// next replaces it.
    pub frame: Option<Arc<RenderImage>>,
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
    /// Sharing their screen.
    pub streaming: bool,
    pub is_self: bool,
}
