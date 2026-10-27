//! Every sender summed and laid out for the speakers.

use std::sync::{Arc, Mutex};

use cpal::{FromSample, SizedSample};
use songbird::constants::SAMPLE_RATE;

use super::ring::SampleRing;
use super::{CHANNELS, Sender};
use crate::platform::audio::sanitize;

/// Soft-limits a mix past full scale: overlapping speakers would otherwise clip
/// into audible harmonics. `tanh` meets the untouched signal at the same slope,
/// so nothing steps at the knee.
fn limit(sample: f32) -> f32 {
    // High enough that a single speaker is never touched.
    const THRESHOLD: f32 = 0.85;
    const HEADROOM: f32 = 1. - THRESHOLD;

    if !sample.is_finite() {
        return 0.;
    }
    if sample.abs() <= THRESHOLD {
        return sample;
    }
    let over = (sample.abs() - THRESHOLD) / HEADROOM;
    (THRESHOLD + HEADROOM * over.tanh()).copysign(sample)
}

/// One ring per sender, so one person's stall doesn't cut out everyone else.
#[derive(Default)]
pub(super) struct Mixer {
    /// A list, not a map: the callback walks all of it and never looks up.
    rings: Mutex<Vec<(Sender, Arc<SampleRing>)>>,
}

impl Mixer {
    /// `stream` is caller-held scratch so the audio thread doesn't allocate.
    fn fill(&self, out: &mut [f32], stream: &mut Vec<f32>) {
        out.fill(0.);
        stream.clear();
        stream.resize(out.len(), 0.);

        if let Ok(rings) = self.rings.lock() {
            for (_, ring) in rings.iter() {
                ring.fill(stream);
                for (slot, sample) in out.iter_mut().zip(stream.iter()) {
                    *slot += sample;
                }
            }
        }

        for slot in out.iter_mut() {
            *slot = limit(*slot);
        }
    }

    pub(super) fn add(&self, sender: Sender, ring: Arc<SampleRing>) {
        if let Ok(mut rings) = self.rings.lock() {
            rings.push((sender, ring));
        }
    }

    pub(super) fn remove(&self, sender: Sender) {
        if let Ok(mut rings) = self.rings.lock() {
            rings.retain(|(id, _)| *id != sender);
        }
    }

    pub(super) fn clear(&self) {
        if let Ok(mut rings) = self.rings.lock() {
            rings.clear();
        }
    }
}

/// Resamples 48kHz stereo to the device. Phase and the two bracketing frames
/// carry across callbacks; restarting each time steps the waveform at every
/// boundary, heard as a tone over the speech.
pub(super) struct Playback {
    mixer: Arc<Mixer>,
    previous: [f32; CHANNELS],
    next: [f32; CHANNELS],
    /// Position between `previous` and `next`, in `[0, 1)`.
    phase: f64,
    step: f64,
    /// Reused so the audio thread stops allocating once buffer size settles.
    scratch: Vec<f32>,
    stream: Vec<f32>,
    channels: usize,
}

impl Playback {
    pub(super) fn new(mixer: Arc<Mixer>, sample_rate: u32, channels: usize) -> Self {
        Self {
            mixer,
            previous: [0.; CHANNELS],
            next: [0.; CHANNELS],
            phase: 0.,
            step: f64::from(SAMPLE_RATE) / f64::from(sample_rate.max(1)),
            scratch: Vec::new(),
            stream: Vec::new(),
            channels: channels.max(1),
        }
    }

    pub(super) fn render<T>(&mut self, out: &mut [T])
    where
        T: SizedSample + FromSample<f32>,
    {
        let frames = out.len() / self.channels;
        if frames == 0 {
            out.fill(T::EQUILIBRIUM);
            return;
        }

        // Counted by repeating the loop's additions; the closed form rounds
        // differently and leaves the resampler a frame off for the rest of the call.
        let mut phase = self.phase;
        let mut pulls = 0;
        for _ in 0..frames {
            phase += self.step;
            while phase >= 1. {
                pulls += 1;
                phase -= 1.;
            }
        }

        self.scratch.clear();
        self.scratch.resize(pulls * CHANNELS, 0.);
        self.mixer.fill(&mut self.scratch, &mut self.stream);

        let mut pulled = 0;
        for frame in 0..frames {
            let blend = self.phase as f32;
            for (channel, slot) in out[frame * self.channels..][..self.channels]
                .iter_mut()
                .enumerate()
            {
                let (previous, next) = match channel {
                    0 | 1 => (self.previous[channel], self.next[channel]),
                    // Surround channels get the stereo mix rather than silence.
                    _ => (
                        (self.previous[0] + self.previous[1]) / 2.,
                        (self.next[0] + self.next[1]) / 2.,
                    ),
                };
                *slot = T::from_sample(sanitize(previous + (next - previous) * blend));
            }

            self.phase += self.step;
            while self.phase >= 1. {
                self.previous = self.next;
                self.next = self
                    .scratch
                    .get(pulled * CHANNELS..(pulled + 1) * CHANNELS)
                    .map_or([0.; CHANNELS], |frame| [frame[0], frame[1]]);
                pulled += 1;
                self.phase -= 1.;
            }
        }

        // Any trailing partial frame.
        out[frames * self.channels..].fill(T::EQUILIBRIUM);
    }
}
