//! Fallback for platforms without a capture backend.

use futures::channel::mpsc::UnboundedSender;

use super::{CaptureEvent, CaptureSettings, Control};

/// Never constructed: there's no picker to produce one.
pub struct CaptureSource {
    _private: (),
}

pub(super) fn is_supported() -> bool {
    false
}

pub(super) fn pick(_window: isize) -> Result<std::future::Ready<Option<CaptureSource>>, String> {
    Err("Screen sharing isn't supported on this platform".into())
}

pub(super) fn capture(
    _source: CaptureSource,
    _settings: CaptureSettings,
    _control: &Control,
    _events: &UnboundedSender<CaptureEvent>,
) -> Result<(), String> {
    Err("Screen sharing isn't supported on this platform".into())
}
