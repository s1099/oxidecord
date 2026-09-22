//! The speakers, for a video's audio track.
//!
//! Separate from the call's output in [`crate::voice`] on purpose: a video is
//! played while a call may be running, and the two want different buffering —
//! a call trades latency away to survive network jitter, a local file has none
//! to survive.
//!
//! cpal streams are `!Send` on some platforms, so the stream lives on a thread
//! of its own that does nothing but hold it open until playback ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SizedSample, StreamConfig};

use super::clock::{AudioRing, CHANNELS};
use crate::platform::audio::{in_device_format, sanitize};

/// How often the holder thread checks whether playback has ended. The stream
/// stops when it is dropped, so this only bounds teardown, and nothing is
/// audible in the meantime.
const STOP_POLL: Duration = Duration::from_millis(50);

/// Opens the default output device and returns the ring the decoder fills.
///
/// The device's own sample rate comes back with it, because the decoder asks
/// Media Foundation to resample to exactly that — letting the OS do it costs
/// nothing and saves carrying a resampler here.
///
/// `None` when there is no usable output device, which is not fatal: the
/// caller falls back to a wall clock and plays the video silently.
pub(super) fn start(stop: Arc<AtomicBool>) -> Option<(Arc<AudioRing>, u32)> {
    let host = cpal::default_host();
    // Picked here, on the caller's thread, because the decoder needs the rate
    // before it can configure its audio output — only the stream is `!Send`.
    let device = host.default_output_device()?;
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

    // The stream stops when it is dropped, so this thread's only job from here
    // is to outlive the playback it belongs to.
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
    // Held across callbacks so the audio thread stops allocating once the
    // device's buffer size has settled.
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
            // Usually the device going away mid-playback, which is worth
            // knowing about when a video has gone silent.
            |err| eprintln!("video audio stream error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

/// Lays the ring's stereo frames out for a device with some other channel
/// count. Anything past stereo gets the downmix, which is better than leaving
/// those speakers silent.
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
