//! Stand-in for platforms with no decoder backend yet.
//!
//! macOS would go through AVFoundation and Linux through GStreamer; both fit
//! the same shape as the Windows one — pull BGRA frames and stereo samples,
//! pace them against the output device — so this is a gap rather than a design
//! limit. Until then the UI keeps the poster card and its "open externally"
//! action, which is what this error drives it back to.

use super::Session;

pub(super) fn decode(_session: Session) -> Result<(), String> {
    Err(String::from(
        "Playing videos in the app isn't supported on this platform yet.",
    ))
}
