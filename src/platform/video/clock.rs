//! What playback is paced against, and the buffer between the decoder and the
//! output device.
//!
//! A video has to be shown at the speed its audio is being heard at, so the
//! clock everything reads is the output device's: [`AudioRing`] counts the
//! frames the callback has actually handed the speakers. Counting what was
//! *played* rather than what was decoded is what makes pausing and starving
//! free — when nothing is coming out the clock stops on its own, and the
//! decoder stops releasing frames without being told to.
//!
//! A file with no audio track has no such clock, so [`Clock::Wall`] stands in
//! with elapsed time and has to be paused explicitly.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Interleaved stereo, which is what the decoder is asked to produce; the
/// output callback lays it out for however many channels the device has.
pub(super) const CHANNELS: usize = 2;

/// Ceiling on the ring.
///
/// Deliberately shallow. It is the decoder's only backpressure, so it doubles
/// as the bound on how far ahead of the speakers decoding may run — and a deep
/// buffer here is both latency on every seek and memory held for no gain.
const MAX_BUFFERED_MS: u32 = 200;

/// Samples between the decoder and the output device, plus the play position
/// derived from what the device has consumed.
pub(super) struct AudioRing {
    samples: Mutex<Vec<f32>>,
    capacity: usize,
    sample_rate: u32,
    /// Where the ring was last seeked to. The play position is this plus
    /// whatever has been played since.
    base_micros: AtomicU64,
    /// Frames played since `base_micros`, counting only frames actually filled
    /// from the ring — silence emitted into an underrun must not advance the
    /// clock, or a stall would be made up for by skipping video.
    frames_since_base: AtomicU64,
    /// While set the callback emits silence without draining, so muting does
    /// not also stop the clock and with it the picture.
    muted: AtomicBool,
}

impl AudioRing {
    pub(super) fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        let capacity = (sample_rate as usize * MAX_BUFFERED_MS as usize / 1000) * CHANNELS;
        Self {
            samples: Mutex::new(Vec::with_capacity(capacity)),
            capacity: capacity.max(CHANNELS),
            sample_rate,
            base_micros: AtomicU64::new(0),
            frames_since_base: AtomicU64::new(0),
            muted: AtomicBool::new(false),
        }
    }

    /// Takes as much of `input` as there is room for and reports how much that
    /// was. Never blocks: the decoder retries the remainder on its next pass,
    /// so it can keep releasing video frames that came due meanwhile.
    pub(super) fn try_push(&self, input: &[f32]) -> usize {
        let Ok(mut samples) = self.samples.lock() else {
            return input.len();
        };
        let room = self.capacity.saturating_sub(samples.len());
        let taken = room.min(input.len());
        samples.extend_from_slice(&input[..taken]);
        taken
    }

    pub(super) fn has_room(&self) -> bool {
        self.samples
            .lock()
            .is_ok_and(|samples| samples.len() < self.capacity)
    }

    /// Whether everything pushed has been played. The decoder waits for this
    /// before declaring the end of a clip, so the last packet is heard rather
    /// than cut off by the teardown that follows.
    pub(super) fn is_drained(&self) -> bool {
        self.samples.lock().is_ok_and(|samples| samples.is_empty())
    }

    /// Fills one output callback with stereo frames, padding with silence when
    /// the decoder has not kept up. Runs on the audio thread, so it takes the
    /// one lock and never allocates.
    pub(super) fn fill(&self, out: &mut [f32]) {
        out.fill(0.);
        if self.muted.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };

        let taken = out.len().min(samples.len()) / CHANNELS * CHANNELS;
        out[..taken].copy_from_slice(&samples[..taken]);
        samples.drain(..taken);

        self.frames_since_base
            .fetch_add((taken / CHANNELS) as u64, Ordering::Relaxed);
    }

    pub(super) fn position(&self) -> Duration {
        let base = self.base_micros.load(Ordering::Relaxed);
        let frames = self.frames_since_base.load(Ordering::Relaxed);
        Duration::from_micros(base + frames * 1_000_000 / u64::from(self.sample_rate))
    }

    /// Drops everything buffered and restarts the clock at `position`. Called
    /// after a seek, where the buffered audio belongs to the old position and
    /// playing it would be heard as a stutter before the jump.
    pub(super) fn reset_to(&self, position: Duration) {
        if let Ok(mut samples) = self.samples.lock() {
            samples.clear();
        }
        self.base_micros
            .store(position.as_micros() as u64, Ordering::Relaxed);
        self.frames_since_base.store(0, Ordering::Relaxed);
    }

    pub(super) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }
}

/// Elapsed time, for a video with no audio track to be paced against.
pub(super) struct WallClock {
    state: Mutex<WallState>,
}

struct WallState {
    /// Position reached before the current run started.
    base: Duration,
    /// When the current run started; `None` while paused.
    running_since: Option<Instant>,
}

/// What the decoder reads to decide when a frame is due.
pub(super) enum Clock {
    /// The output device's, for anything with sound.
    Audio(Arc<AudioRing>),
    /// Elapsed time, for a video with no audio track — or one whose audio
    /// could not be opened, which is still better than refusing to play it.
    Wall(WallClock),
}

impl Clock {
    pub(super) fn wall() -> Self {
        Self::Wall(WallClock {
            state: Mutex::new(WallState {
                base: Duration::ZERO,
                running_since: Some(Instant::now()),
            }),
        })
    }

    pub(super) fn position(&self) -> Duration {
        match self {
            Self::Audio(ring) => ring.position(),
            Self::Wall(wall) => {
                let Ok(state) = wall.state.lock() else {
                    return Duration::ZERO;
                };
                match state.running_since {
                    Some(since) => state.base + since.elapsed(),
                    None => state.base,
                }
            }
        }
    }

    /// An audio clock pauses by itself — the ring stops being drained, so it
    /// stops advancing — but a wall clock has to be told.
    pub(super) fn set_paused(&self, paused: bool) {
        let Self::Wall(wall) = self else {
            return;
        };
        let Ok(mut state) = wall.state.lock() else {
            return;
        };
        match (paused, state.running_since) {
            (true, Some(since)) => {
                state.base += since.elapsed();
                state.running_since = None;
            }
            (false, None) => state.running_since = Some(Instant::now()),
            _ => {}
        }
    }

    pub(super) fn reset_to(&self, position: Duration) {
        match self {
            Self::Audio(ring) => ring.reset_to(position),
            Self::Wall(wall) => {
                if let Ok(mut state) = wall.state.lock() {
                    state.base = position;
                    if state.running_since.is_some() {
                        state.running_since = Some(Instant::now());
                    }
                }
            }
        }
    }
}
