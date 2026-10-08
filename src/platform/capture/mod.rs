//! Screen capture for Go Live, encoded by the operating system.
//!
//! Like [`super::video`], nothing here ships a codec. Windows captures through
//! Windows.Graphics.Capture and encodes through Media Foundation's H.264
//! encoder — the GPU's own where the machine has one — so a stream costs a
//! few percent of a core rather than the whole of one, and there's no codec
//! licensing to carry. Other platforms fall back to `unsupported.rs`, which
//! reports that capture isn't available.
//!
//! A source is picked first ([`pick_source`]), with the system's own picker.
//! One [`ScreenCapture`] then drives it on a thread of its own, reporting
//! encoded frames back over a channel as [`CaptureEvent`]s. The output size is
//! fixed when the capture starts; a window resized afterwards is scaled to fit
//! inside it rather than changing the stream's resolution under the viewers.
//!
//! Dropping the capture stops the thread.

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub use backend::CaptureSource;

/// How a capture is encoded.
#[derive(Clone, Copy)]
pub struct CaptureSettings {
    /// The largest frame sent. A smaller source is sent at its own size.
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    /// Target bitrate, in bits per second.
    pub bitrate: u32,
}

/// One encoded frame: an H.264 access unit in Annex B form, start codes and
/// all, which is what both the DAVE encryptor and the RTP packetizer read.
pub struct EncodedFrame {
    pub data: Vec<u8>,
    /// Whether it can be decoded on its own. Keyframes carry their parameter
    /// sets, so a viewer joining on one has everything they need.
    pub keyframe: bool,
    /// When the frame was captured, from the start of the capture.
    pub timestamp: Duration,
}

/// The NAL units in an Annex B buffer, start codes stripped.
///
/// Split the way WebRTC splits them: a three-byte start code preceded by a
/// zero is a four-byte one, and that one zero is all that's taken. Anything
/// more would cut into the unit before, and a DAVE-encrypted unit can
/// legitimately end in zeros — the receiver's decryptor expects them back.
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    // (where the start code begins, where the unit after it begins)
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

/// What a running capture reports back.
pub enum CaptureEvent {
    /// The source is open and frames will follow at this size.
    Started {
        width: u32,
        height: u32,
    },
    Frame(EncodedFrame),
    /// The source went away — the window was closed, or the display
    /// unplugged — and the capture has stopped.
    SourceClosed,
    /// Capture or encoding failed, and the capture has stopped.
    Failed(String),
}

/// State shared with the capture thread. Atomics, because the thread checks
/// them every frame and must never wait on the caller.
struct Control {
    stopped: AtomicBool,
    keyframe: AtomicBool,
}

impl Control {
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Claims a pending keyframe request, leaving none behind.
    fn take_keyframe(&self) -> bool {
        self.keyframe.swap(false, Ordering::AcqRel)
    }
}

/// Whether this machine can capture its screen at all.
pub fn is_supported() -> bool {
    backend::is_supported()
}

/// Opens the system's capture picker over `window` and resolves to whatever
/// the user chose, or `None` if they cancelled.
///
/// The picker is modal to the window it's opened over, so the handle is read
/// here, while the window is still borrowed; the future that comes back holds
/// nothing of it.
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

/// One running capture.
pub struct ScreenCapture {
    control: Arc<Control>,
}

impl ScreenCapture {
    /// Starts capturing `source` on a thread of its own. Events are delivered
    /// on `events` until the capture stops or the receiver is dropped.
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

    /// Asks for the next frame to be a keyframe, for a viewer who has just
    /// joined or lost too much to recover from.
    pub fn request_keyframe(&self) {
        self.control.keyframe.store(true, Ordering::Release);
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        // Not joined: the thread notices within a frame and tears down on its
        // own, and the caller is an async task that mustn't block on it.
        self.control.stopped.store(true, Ordering::Release);
    }
}
