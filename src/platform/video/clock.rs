//! The playback clock and the buffer between the decoder and the output device.
//!
//! Video is paced against the output device: [`AudioRing`] counts frames the
//! callback actually handed the speakers. Counting what was played, not
//! decoded, makes pausing and starving free, since the clock stops by itself.
//! A file with no audio uses [`Clock::Wall`], which must be paused explicitly.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Interleaved stereo; the output callback adapts it to the device's channels.
pub(super) const CHANNELS: usize = 2;

/// Deliberately shallow: it's the decoder's only backpressure, and a deep
/// buffer adds latency on every seek.
const MAX_BUFFERED_MS: u32 = 200;

pub(super) struct AudioRing {
    samples: Mutex<VecDeque<f32>>,
    capacity: usize,
    sample_rate: u32,
    /// Where the ring was last seeked to.
    base_micros: AtomicU64,
    /// Only frames filled from the ring count; underrun silence must not
    /// advance the clock, or a stall would be made up by skipping video.
    frames_since_base: AtomicU64,
    /// Emits silence without draining, so muting doesn't stop the clock and the picture.
    muted: AtomicBool,
}

impl AudioRing {
    pub(super) fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        let capacity = (sample_rate as usize * MAX_BUFFERED_MS as usize / 1000) * CHANNELS;
        Self {
            samples: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity: capacity.max(CHANNELS),
            sample_rate,
            base_micros: AtomicU64::new(0),
            frames_since_base: AtomicU64::new(0),
            muted: AtomicBool::new(false),
        }
    }

    /// Never blocks: the decoder retries the remainder next pass so it can keep
    /// releasing video frames that came due.
    pub(super) fn try_push(&self, input: &[f32]) -> usize {
        let Ok(mut samples) = self.samples.lock() else {
            return input.len();
        };
        let room = self.capacity.saturating_sub(samples.len());
        let taken = room.min(input.len());
        samples.extend(&input[..taken]);
        taken
    }

    pub(super) fn has_room(&self) -> bool {
        self.samples
            .lock()
            .is_ok_and(|samples| samples.len() < self.capacity)
    }

    /// The decoder waits for this before ending a clip so the last packet isn't cut off.
    pub(super) fn is_drained(&self) -> bool {
        self.samples.lock().is_ok_and(|samples| samples.is_empty())
    }

    /// Runs on the audio thread: takes the one lock and never allocates.
    pub(super) fn fill(&self, out: &mut [f32]) {
        out.fill(0.);
        if self.muted.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };

        let taken = out.len().min(samples.len()) / CHANNELS * CHANNELS;
        for (slot, sample) in out[..taken].iter_mut().zip(samples.drain(..taken)) {
            *slot = sample;
        }

        self.frames_since_base
            .fetch_add((taken / CHANNELS) as u64, Ordering::Relaxed);
    }

    pub(super) fn position(&self) -> Duration {
        let base = self.base_micros.load(Ordering::Relaxed);
        let frames = self.frames_since_base.load(Ordering::Relaxed);
        Duration::from_micros(base + frames * 1_000_000 / u64::from(self.sample_rate))
    }

    /// For a seek: buffered audio belongs to the old position and would stutter before the jump.
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

pub(super) enum Clock {
    Audio(Arc<AudioRing>),
    /// For a video with no audio track, or whose audio couldn't be opened.
    Wall {
        base: Duration,
        /// `None` while paused.
        running_since: Option<Instant>,
    },
}

impl Clock {
    pub(super) fn wall() -> Self {
        Self::Wall {
            base: Duration::ZERO,
            running_since: Some(Instant::now()),
        }
    }

    pub(super) fn position(&self) -> Duration {
        match self {
            Self::Audio(ring) => ring.position(),
            Self::Wall {
                base,
                running_since,
            } => *base + running_since.map_or(Duration::ZERO, |since| since.elapsed()),
        }
    }

    /// An audio clock pauses by itself (the ring stops draining); a wall clock must be told.
    pub(super) fn set_paused(&mut self, paused: bool) {
        let Self::Wall {
            base,
            running_since,
        } = self
        else {
            return;
        };
        match (paused, *running_since) {
            (true, Some(since)) => {
                *base += since.elapsed();
                *running_since = None;
            }
            (false, None) => *running_since = Some(Instant::now()),
            _ => {}
        }
    }

    pub(super) fn reset_to(&mut self, position: Duration) {
        match self {
            Self::Audio(ring) => ring.reset_to(position),
            Self::Wall {
                base,
                running_since,
            } => {
                *base = position;
                if running_since.is_some() {
                    *running_since = Some(Instant::now());
                }
            }
        }
    }
}
