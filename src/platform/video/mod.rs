//! Inline video playback, decoded by the operating system.
//!
//! No codec ships here: Windows decodes through Media Foundation (hardware
//! acceleration, no redistributables or licensing).
//!
//! A [`VideoPlayer`] downloads the clip, opens the speakers and decodes on its
//! own thread, reporting [`VideoEvent`]s. Frames are BGRA pre-scaled to the
//! drawn size, which is what keeps this cheap: a few hundred kilobytes rather
//! than eight megabytes for 1080p. Dropping the player stops the thread, closes
//! the device and deletes the downloaded file.

mod clock;
mod output;

#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(not(windows), path = "unsupported.rs")]
mod backend;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;

use super::{http, runtime};
use clock::AudioRing;

/// Decoded frames in flight to the UI before the decoder starts dropping them.
/// Small on purpose: an undrawn frame is memory held for a stale picture, and a
/// decoder further ahead means the UI can't keep up, where skipping beats buffering.
const MAX_FRAMES_IN_FLIGHT: usize = 3;

/// Sentinel for no pending seek; a real position can't reach it.
const NO_SEEK: u64 = u64::MAX;

pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Tightly packed BGRA, top row first: what [`gpui_kit::RenderImage`] takes.
    pub bgra: Vec<u8>,
    pub position: Duration,
}

pub enum VideoEvent {
    Opened {
        duration: Duration,
    },
    Frame(VideoFrame),
    Ended,
    /// The download failed, or nothing on the machine could decode the file.
    Failed(String),
}

/// Atomics rather than a lock: the decoder reads these every pass and the audio
/// callback reads the mute flag, and neither may wait on a mutex the UI holds.
struct Control {
    stopped: AtomicBool,
    paused: AtomicBool,
    muted: AtomicBool,
    /// Milliseconds, or [`NO_SEEK`].
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

    /// Only the decoder calls this, so the swap can't lose a request to another reader.
    fn take_seek(&self) -> Option<Duration> {
        match self.seek_ms.swap(NO_SEEK, Ordering::AcqRel) {
            NO_SEEK => None,
            ms => Some(Duration::from_millis(ms)),
        }
    }

    /// Reserves a slot when it returns true.
    fn reserve_frame(&self) -> bool {
        self.frames_in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |in_flight| {
                (in_flight < MAX_FRAMES_IN_FLIGHT).then_some(in_flight + 1)
            })
            .is_ok()
    }
}

/// Dropping it ends playback.
pub struct VideoPlayer {
    control: Arc<Control>,
}

impl VideoPlayer {
    /// Returns at once; the download and decode run on the player's thread.
    /// `target` is in physical pixels.
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

    /// Only the latest request is honoured, so dragging the bar doesn't queue a seek per pixel.
    pub fn seek(&self, position: Duration) {
        let ms = (position.as_millis() as u64).min(NO_SEEK - 1);
        self.control.seek_ms.store(ms, Ordering::Release);
    }

    /// Frees a slot; without it the decoder stalls after [`MAX_FRAMES_IN_FLIGHT`] frames.
    pub fn frame_consumed(&self) {
        self.control.frames_in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
    }
}

struct Session<'a> {
    /// Decoded from disk, not streamed: Discord's CDN links are signed and
    /// redirected, which OS media sources handle inconsistently.
    pub path: &'a Path,
    /// Physical pixels.
    pub target: (u32, u32),
    pub control: &'a Control,
    /// Speakers and their rate; `None` plays silently.
    pub audio: Option<(Arc<AudioRing>, u32)>,
    /// Returns whether anyone is still listening; decoding stops once not.
    pub emit: &'a mut dyn FnMut(VideoEvent) -> bool,
}

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

    // Shared with the device thread so closing the player closes the stream too.
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

/// The whole file lands before playback so the decoder gets a seekable local
/// file; scrubbing then needs no byte-range support.
fn fetch_to_temp(url: &str, control: &Control) -> Result<PathBuf, String> {
    let path = temp_path("video");
    // Fine to block: this is the decoder's own thread.
    let result = runtime::handle().block_on(download(url.to_owned(), path.clone(), || {
        control.is_stopped()
    }));
    result.map(|()| path)
}

/// Unique per call so two clips can't collide.
fn temp_path(kind: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    std::env::temp_dir().join(format!("oxidecord-{kind}-{unique:x}"))
}

/// Streams so a big clip isn't held in memory and `cancelled` can stop it. Leaves no file on
/// failure.
async fn download(url: String, path: PathBuf, cancelled: impl Fn() -> bool) -> Result<(), String> {
    let result = async {
        let mut file = std::fs::File::create(&path)
            .map_err(|err| format!("Couldn't buffer the video: {err}"))?;
        let download = |err: reqwest::Error| format!("Couldn't download the video: {err}");
        let mut response = http::client()
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(download)?;
        while let Some(chunk) = response.chunk().await.map_err(download)? {
            if cancelled() {
                return Err(String::from("cancelled"));
            }
            file.write_all(&chunk)
                .map_err(|err| format!("Couldn't buffer the video: {err}"))?;
        }
        Ok(())
    }
    .await;

    if result.is_err() {
        _ = std::fs::remove_file(&path);
    }
    result
}

/// A short clip decoded whole, looped like a GIF.
///
/// Discord serves most GIFs (Tenor, Klipy, Giphy) as muted mp4s in a `gifv`
/// embed and loops them instead of offering a player.
pub struct LoopingClip {
    pub frames: Vec<VideoFrame>,
    /// The last frame is held until this. Zero when the file doesn't say.
    pub duration: Duration,
}

/// Frames stay in memory while on screen, so a long clip is thinned out rather
/// than letting a channel full of them add up. A few seconds at full rate.
const LOOP_BUDGET: usize = 24 * 1024 * 1024;

/// `target` is in physical pixels. The decode runs on whichever thread awaits
/// this, so call it from a background executor.
pub async fn decode_loop(url: String, target: (u32, u32)) -> Result<LoopingClip, String> {
    let path = temp_path("gif");
    runtime::run(download(url, path.clone(), || false))
        .await
        .map_err(|err| format!("Couldn't download the video: {err}"))??;

    let result = backend::decode_all(&path, target, LOOP_BUDGET);
    let _ = std::fs::remove_file(&path);
    let (frames, duration) = result?;
    Ok(LoopingClip { frames, duration })
}
