//! The speakers for a video's audio track.
//!
//! Separate from the call's output in [`crate::voice`]: a call trades latency
//! for network jitter tolerance, a local file has no jitter. cpal streams are
//! `!Send` on some platforms, so a thread just holds the stream open.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait as _, StreamTrait as _};
use cpal::{FromSample, SizedSample, StreamConfig};

use super::clock::{AudioRing, CHANNELS};
use crate::platform::audio::{chosen_output, in_device_format, sanitize};
use crate::platform::prefs;

/// Only bounds teardown latency; the stream stops when dropped.
const STOP_POLL: Duration = Duration::from_millis(50);

/// Returns the ring the decoder fills and the device's sample rate, which the
/// decoder has Media Foundation resample to so no resampler is needed here.
/// `None` without a usable device; the caller then plays silently.
pub(super) fn start(stop: Arc<AtomicBool>) -> Option<(Arc<AudioRing>, u32)> {
    let host = cpal::default_host();
    // Picked on the caller's thread because the decoder needs the rate to
    // configure its audio; only the stream is `!Send`. Read per clip so a
    // device switch applies.
    let chosen = prefs::load().output_device;
    let device = chosen_output(&host, chosen.as_deref())?;
    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate();

    let ring = Arc::new(AudioRing::new(sample_rate));

    let held = ring.clone();
    std::thread::Builder::new()
        .name("video-audio".into())
        .spawn(move || run(&device, config, &held, &stop))
        .ok()?;

    Some((ring, sample_rate))
}

fn run(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    ring: &Arc<AudioRing>,
    stop: &AtomicBool,
) {
    let channels = usize::from(config.channels());
    let stream = in_device_format!(
        config.sample_format(),
        build,
        device,
        config.into(),
        channels,
        ring
    );
    let Some(stream) = stream else {
        return;
    };

    // The stream stops when dropped, so this thread just has to outlive playback.
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(STOP_POLL);
    }
    drop(stream);
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    channels: usize,
    ring: &Arc<AudioRing>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let ring = ring.clone();
    let channels = channels.max(1);
    // Reused so the audio thread stops allocating once the buffer size settles.
    let mut scratch: Vec<f32> = Vec::new();

    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &_| {
                let frames = out.len() / channels;
                scratch.clear();
                scratch.resize(frames * CHANNELS, 0.);
                ring.fill(&mut scratch);
                spread(&scratch, out, channels);
            },
            // Usually the device going away mid-playback.
            |err| eprintln!("video audio stream error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

/// Channels past stereo get the downmix rather than silence.
fn spread<T>(stereo: &[f32], out: &mut [T], channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    for (frame, slot) in stereo.chunks_exact(CHANNELS).zip(out.chunks_mut(channels)) {
        let mix = (frame[0] + frame[1]) / 2.;
        for (channel, sample) in slot.iter_mut().enumerate() {
            let value = match (channels, channel) {
                (1, _) => mix,
                (_, 0 | 1) => frame[channel],
                _ => mix,
            };
            *sample = T::from_sample(sanitize(value));
        }
    }
}
