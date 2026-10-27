//! Every sender summed and laid out for the speakers.

use std::sync::{Arc, Mutex};

use cpal::{FromSample, SizedSample};
use songbird::constants::SAMPLE_RATE;

use super::ring::SampleRing;
use super::{CHANNELS, Sender};
use crate::platform::audio::sanitize;

/// Eases a mix that has run past full scale back inside it.
///
/// Several people talking at once sum past the ceiling, and clipping there
/// turns a loud moment into a burst of harmonics — which is the artefact that
/// actually gets noticed. A knee above `THRESHOLD` bends the peaks instead;
/// `tanh` is chosen for meeting the untouched signal at the same slope, so
/// ordinary speech passes through unaltered and nothing steps as it crosses
/// over.
fn limit(sample: f32) -> f32 {
    // High enough that one person talking, however loudly, is never touched:
    // the knee is there for overlapping speakers, and a waveshaper that
    // reaches into ordinary speech colours it for no gain.
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

/// Everyone audible in the call, one ring per sender, summed on the way out.
///
/// Each sender's packets keep their own clock and their own jitter, so each
/// gets a ring to prime, starve and ramp on its own: one person's connection
/// stalling doesn't cut out everyone else.
#[derive(Default)]
pub(super) struct Mixer {
    /// Keyed by sender. A list rather than a map because the audio callback
    /// walks all of it every time and looks nothing up.
    rings: Mutex<Vec<(Sender, Arc<SampleRing>)>>,
}

impl Mixer {
    /// Fills `out` with every sender summed. `stream` is scratch space for
    /// one sender's share, held by the caller so the audio thread doesn't
    /// allocate.
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

/// Lays the call's 48kHz stereo out in the output device's rate and channel
/// count.
///
/// The position between source frames is carried across callbacks, and so are
/// the two frames it sits between. Resampling each callback from scratch put a
/// step in the waveform at every buffer boundary, and a step that repeats at
/// the callback rate is heard as a tone over the speech rather than as the
/// glitches it is.
pub(super) struct Playback {
    mixer: Arc<Mixer>,
    /// The source frames the next output frame falls between.
    previous: [f32; CHANNELS],
    next: [f32; CHANNELS],
    /// Where between them it falls, in `[0, 1)`.
    phase: f64,
    /// Source frames advanced per output frame.
    step: f64,
    /// The source frames one callback needs. Held across callbacks so the
    /// audio thread stops allocating once the buffer size has settled.
    scratch: Vec<f32>,
    /// One sender's share of `scratch`, for the mixer to sum from.
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

        // How many source frames the loop below will step through. Counted by
        // walking the same additions in the same order rather than by the
        // closed form, which rounds differently often enough to leave the
        // resampler a frame out of step for the rest of the call.
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
                    // Anything past stereo (a surround device) gets the mix,
                    // which is better than leaving those speakers silent.
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

        // Whatever the device asked for beyond a whole number of frames.
        out[frames * self.channels..].fill(T::EQUILIBRIUM);
    }
}
