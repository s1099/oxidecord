//! Inline video playback, decoded by the operating system.
//!
//! Nothing here ships a codec. Windows decodes through Media Foundation, which
//! means hardware acceleration where the machine has it, no redistributable
//! binaries, and no codec licensing — the point being that playing a clip in a
//! message should not cost what embedding a browser engine would.
//!
//! One [`VideoPlayer`] drives one clip. It downloads the attachment, opens the
//! speakers, and runs the decoder on a thread of its own, reporting back over
//! a channel as [`VideoEvent`]s. Frames arrive as tightly packed BGRA already
//! scaled to the size they will be drawn at, which is what makes this cheap:
//! at the size a message renders one, a frame is a few hundred kilobytes
//! rather than the eight megabytes a 1080p frame would be.
//!
//! Dropping the player stops the thread, closes the device, and deletes the
//! downloaded file.

mod clock;
mod output;

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;

use super::runtime;
use clock::AudioRing;

/// How many decoded frames may be in flight to the UI before the decoder
/// starts dropping them.
///
/// Small on purpose. A frame the foreground has not drawn yet is memory held
/// for a picture already out of date, and a decoder running further ahead than
/// this is one the UI cannot keep up with, where skipping is the right answer
/// rather than buffering.
const MAX_FRAMES_IN_FLIGHT: usize = 3;

/// Sentinel for [`Control::seek_ms`] meaning no seek is pending. A real
/// position can never reach it.
const NO_SEEK: u64 = u64::MAX;

/// One decoded frame, ready to hand to gpui.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGRA, top row first — the layout [`gpui::RenderImage`]
    /// takes, so there is no conversion left to do on the foreground.
    pub bgra: Vec<u8>,
    /// Where this frame sits in the clip, for the progress bar.
    pub position: Duration,
}

/// What a running player reports back.
pub enum VideoEvent {
    /// The file is open and decoding has begun; the run time is now known.
    Opened {
        duration: Duration,
    },
    Frame(VideoFrame),
    /// Playback reached the end of the file.
    Ended,
    /// The download failed, or nothing on the machine could decode the file.
    Failed(String),
}

/// Playback state shared with the decoder thread.
///
/// All of it is atomics rather than a lock: the decoder reads these on every
/// pass through its loop, and the audio callback reads the mute flag, where
/// waiting on a mutex held by the UI thread is the one thing that must not
/// happen.
struct Control {
    stopped: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    /// Pending seek target in milliseconds, or [`NO_SEEK`].
    seek_ms: AtomicU64,
    frames_in_flight: AtomicUsize,
}

impl Control {
    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Acquire)
    }

    /// Claims the pending seek, if there is one, leaving none behind. Only the
    /// decoder calls this, so the swap cannot lose a request to a race with
    /// another reader.
    fn take_seek(&self) -> Option<Duration> {
        match self.seek_ms.swap(NO_SEEK, Ordering::AcqRel) {
            NO_SEEK => None,
            ms => Some(Duration::from_millis(ms)),
        }
    }

    /// Whether another frame may be sent, reserving a slot when it may.
    fn reserve_frame(&self) -> bool {
        self.frames_in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |in_flight| {
                (in_flight < MAX_FRAMES_IN_FLIGHT).then_some(in_flight + 1)
            })
            .is_ok()
    }
}

/// A clip being played.
///
/// Dropping it ends playback: the decoder thread sees the stop flag, the
/// output device is closed, and the downloaded file is deleted.
pub struct VideoPlayer {
    control: Arc<Control>,
}

impl VideoPlayer {
    /// Starts playing `url`, scaling frames to fit `target` (in physical
    /// pixels). Events arrive on `events` until playback ends or the player is
    /// dropped.
    ///
    /// Returns immediately — the download and the decoder both run on the
    /// player's own thread, and the UI shows the poster until the first frame
    /// lands.
    pub fn open(url: String, target: (u32, u32), events: UnboundedSender<VideoEvent>) -> Self {
        let control = Arc::new(Control {
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            seek_ms: AtomicU64::new(NO_SEEK),
            frames_in_flight: AtomicUsize::new(0),
        });

        let thread_control = control.clone();
        let thread_events = events.clone();
        let spawned = std::thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || play(url, target, &thread_control, &thread_events));

        if spawned.is_err() {
            let _ = events.unbounded_send(VideoEvent::Failed(
                "Couldn't start the video decoder.".into(),
            ));
        }

        Self { control }
    }

    pub fn set_paused(&self, paused: bool) {
        self.control.paused.store(paused, Ordering::Release);
    }

    pub fn is_paused(&self) -> bool {
        self.control.is_paused()
    }

    pub fn set_muted(&self, muted: bool) {
        self.control.muted.store(muted, Ordering::Release);
    }

    pub fn is_muted(&self) -> bool {
        self.control.is_muted()
    }

    /// Asks for playback to jump to `position`. Only the most recent request
    /// is honoured, so dragging the progress bar doesn't queue up a seek per
    /// pixel moved.
    pub fn seek(&self, position: Duration) {
        let ms = (position.as_millis() as u64).min(NO_SEEK - 1);
        self.control.seek_ms.store(ms, Ordering::Release);
    }

    /// Reports that a frame has been drawn and its memory released, freeing a
    /// slot for the decoder. Without this the decoder stops after
    /// [`MAX_FRAMES_IN_FLIGHT`] frames.
    pub fn frame_consumed(&self) {
        self.control.frames_in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
    }
}

/// Everything the platform decoder needs for one clip.
struct Session<'a> {
    /// The downloaded file. Decoding from disk rather than streaming the URL
    /// keeps this predictable: Discord's CDN links are signed and redirected,
    /// which the OS media sources handle inconsistently.
    pub path: &'a Path,
    /// Physical pixels the frames are scaled to on the way out.
    pub target: (u32, u32),
    pub control: &'a Control,
    /// The speakers and the rate they run at, when a device opened. `None`
    /// plays the video silently rather than refusing to play it.
    pub audio: Option<(Arc<AudioRing>, u32)>,
    /// Reports progress back to the UI. Returns whether anyone is still
    /// listening; once they are not, decoding stops.
    pub emit: &'a mut dyn FnMut(VideoEvent) -> bool,
}

/// The decoder thread: fetch, open the speakers, decode, then clean up.
fn play(url: String, target: (u32, u32), control: &Control, events: &UnboundedSender<VideoEvent>) {
    let mut emit = |event: VideoEvent| events.unbounded_send(event).is_ok();

    let path = match fetch_to_temp(&url, control) {
        Ok(path) => path,
        Err(reason) => {
            emit(VideoEvent::Failed(reason));
            return;
        }
    };
    if control.is_stopped() {
        let _ = std::fs::remove_file(&path);
        return;
    }

    // The stop flag is shared with the device thread so closing the player
    // closes the stream, not just the decoder.
    let stop = Arc::new(AtomicBool::new(false));
    let audio = output::start(stop.clone());

    let result = backend::decode(Session {
        path: &path,
        target,
        control,
        audio,
        emit: &mut emit,
    });

    stop.store(true, Ordering::Release);
    let _ = std::fs::remove_file(&path);

    match result {
        Ok(()) if !control.is_stopped() => {
            emit(VideoEvent::Ended);
        }
        Err(reason) if !control.is_stopped() => {
            emit(VideoEvent::Failed(reason));
        }
        _ => {}
    }
}

/// Downloads the attachment to a temporary file.
///
/// The whole file lands before playback starts, which costs a moment on a big
/// clip but keeps every later step honest: the decoder gets a seekable local
/// file, so scrubbing works without a byte-range implementation, and Discord's
/// signed URL is only ever handled by the HTTP client that already knows how.
fn fetch_to_temp(url: &str, control: &Control) -> Result<PathBuf, String> {
    let url = url.to_owned();
    // Blocking here is fine: this is the decoder's own thread, and it has
    // nothing to do until the file has arrived.
    let bytes = runtime::handle()
        .block_on(async move {
            let client = reqwest::Client::new();
            let response = client.get(&url).send().await?.error_for_status()?;
            response.bytes().await
        })
        .map_err(|err| format!("Couldn't download the video: {err}"))?;

    if control.is_stopped() {
        return Err(String::from("cancelled"));
    }

    // Unique per player so two clips playing in sequence can't collide, and
    // named so a leaked file is obvious in a temp directory.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let path = std::env::temp_dir().join(format!("oxidecord-video-{unique:x}"));

    std::fs::write(&path, &bytes).map_err(|err| format!("Couldn't buffer the video: {err}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a real file end to end: opens the speakers, runs the backend,
    /// and checks what comes back is something gpui can actually draw.
    ///
    /// Needs a video to point at, so it's opt-in:
    /// `OXIDECORD_TEST_VIDEO=path cargo test --release decodes_a_file`
    #[test]
    fn decodes_a_file() {
        let Ok(path) = std::env::var("OXIDECORD_TEST_VIDEO") else {
            return;
        };

        let control = Control {
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            seek_ms: AtomicU64::new(NO_SEEK),
            frames_in_flight: AtomicUsize::new(0),
        };

        let target = (400u32, 300u32);
        let stop = Arc::new(AtomicBool::new(false));
        let audio = output::start(stop.clone());

        let mut frames = 0usize;
        let mut duration = Duration::ZERO;
        let mut failure = None;

        let result = backend::decode(Session {
            path: std::path::Path::new(&path),
            target,
            control: &control,
            audio,
            emit: &mut |event| {
                match event {
                    VideoEvent::Opened { duration: total } => duration = total,
                    VideoEvent::Frame(frame) => {
                        frames += 1;
                        // Freeing the slot is the UI's job; stand in for it.
                        control.frames_in_flight.fetch_sub(1, Ordering::AcqRel);

                        assert!(
                            frame.width <= target.0 && frame.height <= target.1,
                            "frame {}x{} escaped the {}x{} box it was decoded for",
                            frame.width,
                            frame.height,
                            target.0,
                            target.1
                        );
                        assert_eq!(
                            frame.bgra.len(),
                            (frame.width * frame.height * 4) as usize,
                            "buffer doesn't match the size the frame reports"
                        );
                        // Media Foundation leaves RGB32's fourth byte at zero,
                        // which gpui draws as fully transparent — the backend
                        // has to override it or the video is invisible.
                        assert!(
                            frame.bgra.chunks_exact(4).all(|pixel| pixel[3] == 255),
                            "frames must be opaque or they draw as nothing"
                        );

                        // Two seconds is plenty to prove the loop runs;
                        // decoding the whole file would only make this slow.
                        if frame.position >= Duration::from_secs(2) {
                            control.stopped.store(true, Ordering::Release);
                        }
                    }
                    VideoEvent::Failed(reason) => failure = Some(reason),
                    VideoEvent::Ended => {}
                }
                true
            },
        });
        stop.store(true, Ordering::Release);

        assert!(failure.is_none(), "decoder reported: {failure:?}");
        assert!(result.is_ok(), "decode failed: {result:?}");
        assert!(frames > 0, "no frames were produced");
        assert!(!duration.is_zero(), "no run time was reported");
    }
}
