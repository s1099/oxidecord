//! Stand-in for platforms with no decoder backend. The error sends the UI back
//! to the poster card and its "open externally" action.

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
