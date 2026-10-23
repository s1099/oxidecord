//! Finite original sine motifs, synthesized at the device rate with per-note ramps.

use cpal::{FromSample, SizedSample};

use super::Cue;
use crate::platform::audio::sanitize;

pub(super) struct Renderer {
    cue: Cue,
    sample_rate: u32,
    channels: usize,
    sample: usize,
}

impl Renderer {
    pub(super) fn new(cue: Cue, sample_rate: u32, channels: u16) -> Self {
        Self {
            cue,
            sample_rate: sample_rate.max(1),
            channels: usize::from(channels.max(1)),
            sample: 0,
        }
    }

    fn frames(&self) -> usize {
        (u64::from(self.sample_rate) * 180 / 1000).max(2) as usize
    }

    pub(super) fn finished(&self) -> bool {
        self.sample / self.channels >= self.frames()
    }

    pub(super) fn samples_per_second(&self) -> f64 {
        f64::from(self.sample_rate) * self.channels as f64
    }

    pub(super) fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T]) {
        for sample in out {
            let frame = self.sample / self.channels;
            let value = if frame < self.frames() {
                self.value(frame)
            } else {
                0.
            };
            *sample = T::from_sample(sanitize(value));
            self.sample = self.sample.saturating_add(1);
        }
    }

    fn value(&self, frame: usize) -> f32 {
        let half = self.frames() / 2;
        let note = usize::from(frame >= half);
        let start = if note == 0 { 0 } else { half };
        let length = if note == 0 {
            half
        } else {
            self.frames() - half
        };
        let local = frame - start;
        let frequency = match self.cue {
            Cue::Join => [480., 720.][note],
            Cue::Leave => [600., 360.][note],
            Cue::PeerJoin => [760., 880.][note],
        };
        let ramp = (self.sample_rate as f32 * 0.012).max(1.);
        let envelope = (local as f32 / ramp)
            .min((length - 1 - local) as f32 / ramp)
            .clamp(0., 1.);
        let phase = std::f32::consts::TAU * frequency * local as f32 / self.sample_rate as f32;
        0.08 * envelope * phase.sin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_bounded_ramped_cues_at_device_rates_and_channel_counts() {
        for rate in [8_000, 44_100, 48_000, 96_000, 192_000] {
            for channels in [1, 2, 6, 8] {
                for cue in [Cue::Join, Cue::Leave, Cue::PeerJoin] {
                    let mut renderer = Renderer::new(cue, rate, channels);
                    let frames = (rate * 180 / 1000) as usize;
                    let count = frames * usize::from(channels);
                    // Odd callback sizes exercise frame/channel continuity.
                    let mut samples = vec![1f32; count + 127];
                    for chunk in samples.chunks_mut(137) {
                        renderer.fill(chunk);
                    }
                    assert!(renderer.finished());
                    assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 0.081));
                    assert!(samples[..count].iter().any(|s| s.abs() > 0.04));
                    assert!(samples[count..].iter().all(|s| *s == 0.));
                    for frame in samples[..count].chunks_exact(usize::from(channels)) {
                        assert!(frame.iter().all(|s| *s == frame[0]));
                    }
                    assert_eq!(samples[0], 0.);
                    assert_eq!(samples[count - 1], 0.);
                    let mut after = [1f32; 97];
                    renderer.fill(&mut after);
                    assert_eq!(after, [0.; 97]);
                }
            }
        }
    }
}
