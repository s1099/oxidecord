//! Screen capture for Go Live, encoded by the operating system.
//!
//! Like [`super::video`], no codec ships here: Windows captures through
//! Windows.Graphics.Capture and encodes through Media Foundation's H.264
//! encoder (the GPU's own where available).
//!
//! A source is picked with the system picker ([`pick_source`]); a
//! [`ScreenCapture`] then drives it on its own thread. The output size is fixed
//! at start; a resized window is scaled to fit rather than changing the
//! stream's resolution under the viewers. Dropping the capture stops the thread.

#[cfg_attr(windows, path = "windows/mod.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub use backend::CaptureSource;

#[derive(Clone, Copy)]
pub struct CaptureSettings {
    /// The largest frame sent. A smaller source is sent at its own size.
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    /// Target bitrate, in bits per second.
    pub bitrate: u32,
}

/// An H.264 access unit in Annex B form, which is what both the DAVE encryptor
/// and the RTP packetizer read.
pub struct EncodedFrame {
    pub data: Vec<u8>,
    /// Keyframes carry their parameter sets, so a viewer joining on one can decode.
    pub keyframe: bool,
    /// Since the start of the capture.
    pub timestamp: Duration,
}

/// The NAL units in an Annex B buffer, start codes stripped.
///
/// Split like WebRTC: a three-byte start code preceded by a zero is a four-byte
/// one, and only that one zero is taken. More would cut into the previous unit,
/// and a DAVE-encrypted unit can legitimately end in zeros.
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    // (start code begin, unit begin)
    let mut codes = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i + 2] > 1 {
            i += 3;
        } else if data[i + 2] == 1 && data[i + 1] == 0 && data[i] == 0 {
            let start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            codes.push((start, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }

    codes
        .iter()
        .enumerate()
        .map(|(n, &(_, unit))| {
            let end = codes.get(n + 1).map_or(data.len(), |&(next, _)| next);
            &data[unit..end]
        })
        .filter(|unit| !unit.is_empty())
        .collect()
}

pub enum CaptureEvent {
    Started {
        width: u32,
        height: u32,
    },
    Frame(EncodedFrame),
    /// Window closed or display unplugged; the capture has stopped.
    SourceClosed,
    Failed(String),
}

/// Atomics, because the thread checks them every frame and must never wait on the caller.
struct Control {
    stopped: AtomicBool,
    keyframe: AtomicBool,
}

impl Control {
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn take_keyframe(&self) -> bool {
        self.keyframe.swap(false, Ordering::AcqRel)
    }
}

pub fn is_supported() -> bool {
    backend::is_supported()
}

/// Resolves to `None` if the user cancels. The handle is read here while the
/// window is borrowed, so the returned future holds nothing of it.
pub fn pick_source(
    window: &dyn HasWindowHandle,
) -> Result<impl Future<Output = Option<CaptureSource>> + use<>, String> {
    let handle = window
        .window_handle()
        .map_err(|err| format!("No window to open the picker over: {err}"))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err("Screen sharing isn't supported on this platform".into());
    };
    backend::pick(handle.hwnd.get())
}

pub struct ScreenCapture {
    control: Arc<Control>,
}

impl ScreenCapture {
    pub fn start(
        source: CaptureSource,
        settings: CaptureSettings,
        events: UnboundedSender<CaptureEvent>,
    ) -> Self {
        let control = Arc::new(Control {
            stopped: AtomicBool::new(false),
            keyframe: AtomicBool::new(false),
        });

        let thread_control = control.clone();
        let spawned = std::thread::Builder::new()
            .name("screen-capture".into())
            .spawn(move || {
                let event = match backend::capture(source, settings, &thread_control, &events) {
                    Ok(()) if thread_control.is_stopped() => return,
                    Ok(()) => CaptureEvent::SourceClosed,
                    Err(err) => CaptureEvent::Failed(err),
                };
                let _ = events.unbounded_send(event);
            });
        if let Err(err) = spawned {
            eprintln!("couldn't start the capture thread: {err}");
        }

        Self { control }
    }

    /// For a viewer who just joined or lost too much to recover.
    pub fn request_keyframe(&self) {
        self.control.keyframe.store(true, Ordering::Release);
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        // Not joined: the caller is an async task that mustn't block.
        self.control.stopped.store(true, Ordering::Release);
    }
}
