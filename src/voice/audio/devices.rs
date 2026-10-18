//! Picking and opening the audio devices.

use std::sync::Arc;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{DeviceId, FromSample, Sample as _, SizedSample, StreamConfig, SupportedStreamConfig};

use super::activity::CaptureActivity;
use super::playback::{Mixer, Playback};
use super::ring::SampleRing;
use super::{CHANNELS, Shutdown};
use crate::platform::audio::in_device_format;

/// A microphone the user can pick between, as the menu lists it.
pub struct InputDevice {
    /// Stable across runs and reboots, so it's what a preference stores.
    pub id: String,
    pub name: String,
}

pub fn input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    devices
        .filter_map(|device| {
            Some(InputDevice {
                id: device.id().ok()?.to_string(),
                name: device.description().ok()?.name().to_string(),
            })
        })
        .collect()
}

/// The chosen microphone, or the system default when none is chosen or the
/// chosen one has been unplugged since.
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

/// How many of a microphone's channels the capture ring carries.
///
/// The raw input the mixer is handed can only describe mono or stereo, so a
/// device offering more — a mic array, a capture interface — is read as the
/// stereo pair it starts with rather than refused outright.
pub(super) fn ring_channels(device_channels: u16) -> usize {
    usize::from(device_channels).clamp(1, CHANNELS)
}

/// Opens both streams and holds them until the call ends. Runs on its own
/// thread: cpal streams are `!Send` on some platforms, so they can't be handed
/// back to the caller.
pub(super) fn run_devices(
    input: Option<(cpal::Device, SupportedStreamConfig)>,
    output: Option<(cpal::Device, SupportedStreamConfig)>,
    capture: &Arc<SampleRing>,
    mixer: &Arc<Mixer>,
    shutdown: &Arc<Shutdown>,
    activity: &Arc<CaptureActivity>,
) {
    let input = input.and_then(|(device, config)| {
        build_input(&device, config, capture.clone(), activity.clone())
    });
    let output = output.and_then(|(device, config)| build_output(&device, config, mixer.clone()));

    if input.is_none() && output.is_none() {
        return;
    }

    // Both streams stop when they're dropped, so the thread's only job from
    // here is to stay alive for as long as the call does.
    shutdown.wait();
}

/// Fills the capture ring with what the device hears, as `f32` at the device's
/// own rate; the mixer is told that rate and does the resampling.
fn build_input(
    device: &cpal::Device,
    config: SupportedStreamConfig,
    capture: Arc<SampleRing>,
    activity: Arc<CaptureActivity>,
) -> Option<cpal::Stream> {
    let device_channels = usize::from(config.channels());
    let stream = in_device_format!(
        config.sample_format(),
        input_stream,
        device,
        config.into(),
        device_channels,
        capture,
        activity,
    )?;
    stream.play().ok()?;
    Some(stream)
}

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    device_channels: usize,
    capture: Arc<SampleRing>,
    activity: Arc<CaptureActivity>,
) -> Option<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &_| push_capture(data, device_channels, &capture, &activity),
            // Usually the device going away mid-call, which is worth knowing
            // about when a call has gone one-way.
            |err| eprintln!("microphone stream error: {err}"),
            None,
        )
        .ok()
}

/// Copies one callback's worth of microphone into the ring, keeping only the
/// channels [`ring_channels`] promised the mixer.
fn push_capture<T>(
    data: &[T],
    device_channels: usize,
    capture: &SampleRing,
    activity: &CaptureActivity,
) where
    T: Copy,
    f32: FromSample<T>,
{
    if device_channels == 0 {
        return;
    }
    let mut power = 0.;
    let mut samples = 0;
    let mut measure = |&sample| {
        let sample = f32::from_sample(sample);
        if sample.is_finite() {
            power += f64::from(sample.clamp(-1., 1.)).powi(2);
        }
        samples += 1;
        sample
    };
    let keep = ring_channels(device_channels.try_into().unwrap_or(u16::MAX));
    if device_channels <= keep {
        capture.push(data.iter().map(&mut measure));
    } else {
        capture.push(
            data.chunks_exact(device_channels)
                .flat_map(|frame| &frame[..keep])
                .map(&mut measure),
        );
    }
    activity.observe(power, samples, std::time::Instant::now());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::audio::AudioIo;
    use crate::voice::{Speakers, TickHandler, VoiceEvent};
    use std::sync::Mutex;
    use twilight_model::id::Id;

    fn test_audio() -> Arc<AudioIo> {
        Arc::new(AudioIo {
            capture: Arc::new(SampleRing::new(48_000, 1, 30)),
            mixer: Arc::default(),
            incoming: Mutex::default(),
            shutdown: Arc::default(),
            deafened: Arc::default(),
            capture_format: (48_000, 1),
            activity: Arc::default(),
            muted: std::sync::atomic::AtomicBool::new(false),
        })
    }

    #[test]
    fn local_microphone_lights_shared_participant_state_without_remote_packets() {
        let local = Id::new(42);
        let audio = test_audio();
        let (events, mut received) = futures::channel::mpsc::unbounded();
        let handler = TickHandler {
            audio: Arc::new(Mutex::new(audio.clone())),
            local_user: local,
            speakers: Arc::new(Mutex::new(Speakers::default())),
            events,
        };
        push_capture(&[0.1f32; 960], 1, &audio.capture, &audio.activity);
        handler.update_speaking(std::iter::empty());
        assert!(
            matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users.contains(&local)),
            "microphone activity must reach the participant state used by both voice views"
        );
        handler.update_speaking(std::iter::empty());
        assert!(
            received.try_recv().is_err(),
            "unchanged activity must not repaint every tick"
        );

        let remote = Id::new(43);
        if let Ok(mut speakers) = handler.speakers.lock() {
            speakers.users.insert(7, remote);
        }
        handler.update_speaking([7].into_iter());
        assert!(
            matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users == [local, remote].into())
        );
        audio.set_muted(true);
        push_capture(&[0.1f32; 960], 1, &audio.capture, &audio.activity);
        handler.update_speaking([7].into_iter());
        assert!(
            matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users == [remote].into())
        );
        audio.set_muted(false);
        handler.update_speaking([7].into_iter());
        assert!(
            received.try_recv().is_err(),
            "unmuting must not reuse old microphone activity"
        );

        push_capture(&[0.1f32; 960], 1, &audio.capture, &audio.activity);
        handler.update_speaking(std::iter::empty());
        assert!(
            matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users == [local].into())
        );
        audio.set_deafened(true);
        handler.update_speaking(std::iter::empty());
        assert!(matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users.is_empty()));
        push_capture(&[0.1f32; 960], 1, &audio.capture, &audio.activity);
        audio.set_deafened(false);
        handler.update_speaking(std::iter::empty());
        assert!(received.try_recv().is_err());

        let replacement = test_audio();
        if let Ok(mut current) = handler.audio.lock() {
            *current = replacement.clone();
        }
        push_capture(&[0.1f32; 960], 1, &audio.capture, &audio.activity);
        handler.update_speaking(std::iter::empty());
        assert!(
            received.try_recv().is_err(),
            "the old microphone must no longer drive the indicator"
        );
        push_capture(
            &[0.1f32; 960],
            1,
            &replacement.capture,
            &replacement.activity,
        );
        handler.update_speaking(std::iter::empty());
        assert!(
            matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users == [local].into())
        );
        replacement.stop();
        handler.update_speaking(std::iter::empty());
        assert!(matches!(received.try_recv(), Ok(VoiceEvent::Speaking(users)) if users.is_empty()));
    }

    #[test]
    fn only_transmitted_microphone_channels_contribute_to_activity() {
        let capture = SampleRing::new(44_100, 2, 0);
        let activity = CaptureActivity::default();
        push_capture(&[0i16, 0, i16::MAX, i16::MIN], 4, &capture, &activity);
        assert!(!activity.active(std::time::Instant::now()));
        push_capture(&[0.001f32; 960], 2, &capture, &activity);
        push_capture(&[f32::NAN, f32::INFINITY], 2, &capture, &activity);
        assert!(!activity.active(std::time::Instant::now()));
        push_capture(&[i16::MAX, i16::MIN, 0, 0], 4, &capture, &activity);
        assert!(activity.active(std::time::Instant::now()));
        let mut output = [0.; 8];
        capture.fill(&mut output);
        assert!(output.iter().all(|value| value.is_finite()));
    }
}
