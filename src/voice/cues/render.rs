//! Finite stereo PCM playback with continuous-phase device-rate conversion.

use cpal::{FromSample, SizedSample};

use super::clips::Clip;
use crate::platform::audio::sanitize;

// Discord's useSound hook passes 0.4 to SoundUtils (assets/sounds/provenance.json).
const GAIN: f32 = 0.4;

pub(super) struct Renderer<'a> {
    clip: &'a Clip,
    sample_rate: u32,
    channels: usize,
    sample: u64,
    frames: u64,
}

impl<'a> Renderer<'a> {
    pub(super) fn new(clip: &'a Clip, sample_rate: u32, channels: u16) -> Self {
        let sample_rate = sample_rate.max(1);
        Self {
            clip,
            sample_rate,
            channels: usize::from(channels.max(1)),
            sample: 0,
            frames: (clip.frames() as u64 * u64::from(sample_rate))
                .div_ceil(u64::from(clip.sample_rate)),
        }
    }

    pub(super) fn finished(&self) -> bool {
        self.sample / self.channels as u64 >= self.frames
    }

    pub(super) fn samples_per_second(&self) -> f64 {
        f64::from(self.sample_rate) * self.channels as f64
    }

    pub(super) fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T]) {
        for sample in out {
            let frame = self.sample / self.channels as u64;
            let channel = self.sample % self.channels as u64;
            let value = if frame >= self.frames || channel >= 2 {
                0.
            } else if self.channels == 1 {
                (self.value(frame, 0) + self.value(frame, 1)) * 0.5
            } else {
                self.value(frame, channel as usize)
            };
            *sample = T::from_sample(sanitize(value * GAIN));
            self.sample = self.sample.saturating_add(1);
        }
    }

    fn value(&self, frame: u64, channel: usize) -> f32 {
        // ponytail: linear interpolation has no downsampling anti-alias filter;
        // use a band-limited resampler if device-rate fidelity beyond this is needed.
        let position = frame * u64::from(self.clip.sample_rate);
        let index = (position / u64::from(self.sample_rate)) as usize;
        let fraction = (position % u64::from(self.sample_rate)) as f32 / self.sample_rate as f32;
        let next = (index + 1).min(self.clip.frames() - 1);
        let a = self.clip.samples[index * 2 + channel];
        let b = self.clip.samples[next * 2 + channel];
        a + (b - a) * fraction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::cues::{Cue, clips};

    #[test]
    fn original_clips_terminate_at_device_rates_with_callback_independent_phase() {
        for cue in [Cue::Join, Cue::Leave, Cue::PeerJoin, Cue::PeerLeave] {
            let clip = clips::get(cue).unwrap();
            for rate in [8_000, 44_100, 48_000, 96_000, 192_000] {
                for channels in [1, 2, 6, 8] {
                    let frames = (clip.frames() as u64 * u64::from(rate))
                        .div_ceil(u64::from(clip.sample_rate))
                        as usize;
                    let count = frames * usize::from(channels);
                    let mut renderer = Renderer::new(clip, rate, channels);
                    let mut samples = vec![1f32; count + 127];
                    for chunk in samples[..count - 1].chunks_mut(137) {
                        renderer.fill(chunk);
                    }
                    assert!(!renderer.finished());
                    renderer.fill(&mut samples[count - 1..]);
                    assert!(renderer.finished());
                    assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.));
                    assert!(samples[..count].iter().any(|s| s.abs() > 0.001));
                    assert!(samples[count..].iter().all(|s| *s == 0.));
                    let mut contiguous = vec![1f32; samples.len()];
                    Renderer::new(clip, rate, channels).fill(&mut contiguous);
                    assert_eq!(samples, contiguous);
                    if channels > 2 {
                        for frame in samples[..count].chunks_exact(usize::from(channels)) {
                            assert!(frame[2..].iter().all(|s| *s == 0.));
                        }
                    }
                    let mut after = [1f32; 97];
                    renderer.fill(&mut after);
                    assert_eq!(after, [0.; 97]);
                }
            }
        }
    }

    #[test]
    fn native_rate_preserves_recording_channels_and_only_mono_downmixes() {
        for cue in [Cue::Join, Cue::Leave, Cue::PeerLeave] {
            let clip = clips::get(cue).unwrap();
            let mut stereo = vec![0f32; clip.samples.len()];
            Renderer::new(clip, clip.sample_rate, 2).fill(&mut stereo);
            for (actual, original) in stereo.iter().zip(&clip.samples) {
                assert_eq!(*actual, sanitize(original * GAIN));
            }
            let mut mono = vec![0f32; clip.frames()];
            Renderer::new(clip, clip.sample_rate, 1).fill(&mut mono);
            for (actual, original) in mono.iter().zip(clip.samples.as_chunks::<2>().0) {
                assert_eq!(*actual, sanitize((original[0] + original[1]) * 0.5 * GAIN));
            }
        }
    }

    #[test]
    fn resampling_interpolates_stereo_and_sanitizes_integer_conversion() {
        let clip = Clip {
            samples: vec![0., 1., 1., 0., 0., -1.].into_boxed_slice(),
            sample_rate: 2,
        };
        let mut actual = [0f32; 12];
        Renderer::new(&clip, 4, 2).fill(&mut actual);
        assert_eq!(
            actual,
            [0., 0.4, 0.2, 0.2, 0.4, 0., 0.2, -0.2, 0., -0.4, 0., -0.4]
        );
        let invalid = Clip {
            samples: vec![f32::NAN, f32::INFINITY, -10., 10.].into_boxed_slice(),
            sample_rate: 44_100,
        };
        let mut integers = [0i16; 6];
        Renderer::new(&invalid, 44_100, 2).fill(&mut integers);
        assert_eq!(integers, [0, 0, i16::MIN, i16::MAX, 0, 0]);
        let mut unsigned = [0u16; 6];
        Renderer::new(&invalid, 44_100, 2).fill(&mut unsigned);
        assert_eq!(unsigned, [32768, 32768, 0, u16::MAX, 32768, 32768]);
    }
}
