//! Stand-in for platforms with no decoder backend yet.
//!
//! macOS would go through AVFoundation and Linux through GStreamer; both fit
//! the same shape as the Windows one — pull BGRA frames and stereo samples,
//! pace them against the output device — so this is a gap rather than a design
//! limit. Until then the UI keeps the poster card and its "open externally"
//! action, which is what this error drives it back to.

use std::path::Path;
use std::time::Duration;

use super::{Session, VideoFrame};

const UNSUPPORTED: &str = "Playing videos in the app isn't supported on this platform yet.";

pub(super) fn decode(_session: Session) -> Result<(), String> {
    Err(String::from(UNSUPPORTED))
}

pub(super) fn decode_all(
    _path: &Path,
    _target: (u32, u32),
    _budget: usize,
) -> Result<(Vec<VideoFrame>, Duration), String> {
    Err(String::from(UNSUPPORTED))
}
