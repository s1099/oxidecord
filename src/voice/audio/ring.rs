//! The ring of samples between an audio callback and the call.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::platform::audio::sanitize;

/// Rides out scheduling hiccups without a noticeable growing delay.
const MAX_BUFFERED_MS: u32 = 400;

/// Long enough to soften the step, short enough not to swallow a consonant.
const FADE_MS: u32 = 3;

/// Neither side blocks: a starved reader gets silence and an overrun writer
/// drops the oldest audio, since stalling would stutter the whole call.
///
/// Both leave a step in the waveform, which clicks. So the ring ramps down into
/// a gap and back out, and after a stall waits for `prime` samples before
/// draining again.
pub(super) struct SampleRing {
    state: Mutex<RingState>,
    /// Drains and drops are whole frames; losing one sample would put every
    /// later one on the wrong channel.
    pub(super) channels: usize,
    prime: usize,
    capacity: usize,
    /// In frames.
    fade: usize,
}

#[derive(Default)]
struct RingState {
    samples: VecDeque<f32>,
    /// Reader emits silence until the buffer refills after a stall.
    priming: bool,
    /// Last sample per channel; a gap ramps down from here.
    last: Vec<f32>,
    fade_in: usize,
}

impl SampleRing {
    pub(super) fn new(sample_rate: u32, channels: usize, prime_ms: u32) -> Self {
        let channels = channels.max(1);
        let frames = |ms: u32| (u64::from(sample_rate) * u64::from(ms) / 1000) as usize;
        let prime = frames(prime_ms) * channels;
        Self {
            state: Mutex::new(RingState {
                priming: true,
                last: vec![0.; channels],
                ..RingState::default()
            }),
            channels,
            prime,
            // Must hold more than `prime` or priming never finishes.
            capacity: (frames(MAX_BUFFERED_MS) * channels).max(prime * 2),
            fade: frames(FADE_MS).max(1),
        }
    }

    pub(super) fn push(&self, samples: impl Iterator<Item = f32>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        // NaN or over-range samples become full-scale spikes after format conversion.
        state.samples.extend(samples.map(sanitize));

        if state.samples.len() > self.capacity {
            // Keep the newest audio; skipping forward beats falling behind.
            let excess = state.samples.len() - self.capacity;
            let excess = (excess.div_ceil(self.channels) * self.channels).min(state.samples.len());
            state.samples.drain(..excess);
            // The reader jumps over the dropped audio; ramp in after it.
            state.fade_in = self.fade;
        }
    }

    /// `out` must hold a whole number of frames.
    pub(super) fn fill(&self, out: &mut [f32]) {
        if out.is_empty() {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            out.fill(0.);
            return;
        };

        if state.priming {
            if state.samples.len() < self.prime {
                self.ramp_from_last(&state, out);
                state.last.fill(0.);
                return;
            }
            state.priming = false;
            state.fade_in = self.fade;
        }

        let available = (state.samples.len() / self.channels) * self.channels;
        let take = available.min(out.len());
        for slot in out[..take].iter_mut() {
            *slot = state.samples.pop_front().unwrap_or(0.);
        }

        if take < out.len() {
            // Underrun: fade out and re-prime instead of cutting dead and
            // tearing another gap next callback.
            self.fade_out(&mut out[..take]);
            out[take..].fill(0.);
            state.priming = true;
        }

        if state.fade_in > 0 {
            self.fade_in(out, &mut state.fade_in);
        }

        let frames = out.len() / self.channels;
        let last_frame = &out[(frames - 1) * self.channels..][..self.channels];
        state.last.copy_from_slice(last_frame);
    }

    /// For when there is nothing to play; going straight from the last sample to zero clicks.
    fn ramp_from_last(&self, state: &RingState, out: &mut [f32]) {
        out.fill(0.);
        let span = self.fade.min(out.len() / self.channels);
        for frame in 0..span {
            let gain = 1. - (frame + 1) as f32 / span as f32;
            for (channel, slot) in out[frame * self.channels..][..self.channels]
                .iter_mut()
                .enumerate()
            {
                *slot = state.last[channel] * gain;
            }
        }
    }

    fn fade_out(&self, filled: &mut [f32]) {
        let frames = filled.len() / self.channels;
        let span = self.fade.min(frames);
        for frame in 0..span {
            let gain = 1. - (frame + 1) as f32 / span as f32;
            let start = (frames - span + frame) * self.channels;
            for slot in filled[start..][..self.channels].iter_mut() {
                *slot *= gain;
            }
        }
    }

    /// Continues across callbacks if the ramp outlasts one.
    fn fade_in(&self, out: &mut [f32], remaining: &mut usize) {
        let span = (*remaining).min(out.len() / self.channels);
        for frame in 0..span {
            let done = self.fade - *remaining + frame;
            let gain = (done + 1) as f32 / self.fade as f32;
            for slot in out[frame * self.channels..][..self.channels].iter_mut() {
                *slot *= gain;
            }
        }
        *remaining -= span;
    }

    pub(super) fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.samples.clear();
            state.priming = true;
            state.fade_in = 0;
            state.last.fill(0.);
        }
    }
}
