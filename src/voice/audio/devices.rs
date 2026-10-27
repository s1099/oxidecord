//! Picking and opening the audio devices.

use std::sync::Arc;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{DeviceId, FromSample, Sample as _, SizedSample, StreamConfig, SupportedStreamConfig};

use super::playback::{Mixer, Playback};
use super::ring::SampleRing;
use super::{CHANNELS, Shutdown};
use crate::platform::audio::in_device_format;

pub struct AudioDevice {
    /// Stable across runs and reboots, so it's what a preference stores.
    pub id: String,
    pub name: String,
}

pub fn input_devices() -> Vec<AudioDevice> {
    cpal::default_host()
        .input_devices()
        .map_or_else(|_| Vec::new(), listed)
}

pub fn output_devices() -> Vec<AudioDevice> {
    cpal::default_host()
        .output_devices()
        .map_or_else(|_| Vec::new(), listed)
}

fn listed(devices: impl Iterator<Item = cpal::Device>) -> Vec<AudioDevice> {
    devices
        .filter_map(|device| {
            Some(AudioDevice {
                id: device.id().ok()?.to_string(),
                name: device.description().ok()?.name().to_string(),
            })
        })
        .collect()
}

/// Falls back to the default when none is chosen or it has been unplugged.
pub(super) fn chosen_input(host: &cpal::Host, id: Option<&str>) -> Option<cpal::Device> {
    id.and_then(|id| id.parse::<DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_input_device())
}

pub(super) fn input_config(device: cpal::Device) -> Option<(cpal::Device, SupportedStreamConfig)> {
    let supported = device.default_input_config().ok()?;
    Some((device, supported))
}

pub(super) fn output_config(device: cpal::Device) -> Option<(cpal::Device, SupportedStreamConfig)> {
    let supported = device.default_output_config().ok()?;
    Some((device, supported))
}

/// The raw input handed to the mixer can only describe mono or stereo, so
/// devices with more channels are read as their first pair.
pub(super) fn ring_channels(device_channels: u16) -> usize {
    usize::from(device_channels).clamp(1, CHANNELS)
}

/// Runs on its own thread because cpal streams are `!Send` on some platforms.
pub(super) fn run_devices(
    input: Option<(cpal::Device, SupportedStreamConfig)>,
    output: Option<(cpal::Device, SupportedStreamConfig)>,
    capture: &Arc<SampleRing>,
    mixer: &Arc<Mixer>,
    shutdown: &Arc<Shutdown>,
) {
    let input = input.and_then(|(device, config)| build_input(&device, config, capture.clone()));
    let output = output.and_then(|(device, config)| build_output(&device, config, mixer.clone()));

    if input.is_none() && output.is_none() {
        return;
    }

    // Streams stop when dropped, so just stay alive for the call.
    shutdown.wait();
}

fn build_input(
    device: &cpal::Device,
    config: SupportedStreamConfig,
    capture: Arc<SampleRing>,
) -> Option<cpal::Stream> {
    let device_channels = usize::from(config.channels());
    let stream = in_device_format!(
        config.sample_format(),
        input_stream,
        device,
        config.into(),
        device_channels,
        capture,
    )?;
    stream.play().ok()?;
    Some(stream)
}

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    device_channels: usize,
    capture: Arc<SampleRing>,
) -> Option<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &_| push_capture(data, device_channels, &capture),
            // Usually the device vanishing mid-call.
            |err| eprintln!("microphone stream error: {err}"),
            None,
        )
        .ok()
}

fn push_capture<T>(data: &[T], device_channels: usize, capture: &SampleRing)
where
    T: Copy,
    f32: FromSample<T>,
{
    let keep = ring_channels(device_channels.try_into().unwrap_or(u16::MAX));
    if device_channels <= keep {
        capture.push(data.iter().map(|&sample| f32::from_sample(sample)));
    } else {
        capture.push(
            data.chunks_exact(device_channels)
                .flat_map(|frame| &frame[..keep])
                .map(|&sample| f32::from_sample(sample)),
        );
    }
}

fn build_output(
    device: &cpal::Device,
    config: SupportedStreamConfig,
    mixer: Arc<Mixer>,
) -> Option<cpal::Stream> {
    let stream = in_device_format!(
        config.sample_format(),
        output_stream,
        device,
        config.into(),
        mixer,
    )?;
    stream.play().ok()?;
    Some(stream)
}

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mixer: Arc<Mixer>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut render = Playback::new(mixer, config.sample_rate, usize::from(config.channels));

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &_| render.render(data),
            |err| eprintln!("speaker stream error: {err}"),
            None,
        )
        .ok()
}
