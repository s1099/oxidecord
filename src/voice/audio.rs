//! The microphone and the speakers.
//!
//! Two rings of interleaved `f32` sit between the audio devices and the call:
//! capture, which the mixer drains through [`MicSource`], and playback, which
//! the call fills from decoded packets and the output device drains.
//!
//! Capture stays at the device's own sample rate — songbird's mixer resamples
//! whatever an input declares, so there's nothing to convert on the way in.
//! Playback is the other way round: decoded packets are always 48kHz stereo,
//! and the output stream lays them out for the device.
//!
//! Both directions do convert sample formats, to `f32` and back: the format a
//! device reports is whichever one its driver prefers, and a call with no
//! microphone is worse than a handful of conversions.
//!
//! cpal streams are not `Send` on every platform, so both live on a thread of
//! their own that does nothing but hold them open until the call ends.

use std::collections::VecDeque;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{
    DeviceId, FromSample, Sample as _, SampleFormat, SizedSample, StreamConfig,
    SupportedStreamConfig,
};
use songbird::constants::{SAMPLE_RATE, STEREO_FRAME_SIZE};
use songbird::input::core::io::MediaSource;
use songbird::input::{Input, RawAdapter};

/// Decoded packets are stereo, which is what songbird's own frame size counts.
const CHANNELS: usize = 2;

/// Ceiling on each ring, in samples: twenty of songbird's 20ms stereo frames,
/// or 400ms. Enough to ride out a scheduling hiccup, short enough that
/// recovery isn't audible as a growing delay.
const MAX_BUFFERED: usize = STEREO_FRAME_SIZE * 20;

/// A ring of interleaved stereo samples shared between an audio callback and
/// the call.
///
/// Neither side ever blocks: a starved reader gets silence and an overrun
/// writer drops the oldest audio, since stalling either thread would be heard
/// as a stutter across the whole call.
#[derive(Default)]
struct SampleRing {
    samples: Mutex<VecDeque<f32>>,
}

impl SampleRing {
    fn push(&self, samples: impl Iterator<Item = f32>) {
        let Ok(mut buffer) = self.samples.lock() else {
            return;
        };
        buffer.extend(samples);
        // Keep the newest audio: the listener would rather skip forward than
        // fall further behind the conversation.
        if buffer.len() > MAX_BUFFERED {
            let excess = buffer.len() - MAX_BUFFERED;
            buffer.drain(..excess);
        }
    }

    /// Fills `out` with what's buffered, padding with silence when the writer
    /// hasn't kept up.
    fn fill(&self, out: &mut [f32]) {
        let Ok(mut buffer) = self.samples.lock() else {
            out.fill(0.);
            return;
        };
        for slot in out.iter_mut() {
            *slot = buffer.pop_front().unwrap_or(0.);
        }
    }

    fn clear(&self) {
        if let Ok(mut buffer) = self.samples.lock() {
            buffer.clear();
        }
    }
}

/// The audio devices for one call.
pub struct AudioIo {
    capture: Arc<SampleRing>,
    playback: Arc<SampleRing>,
    /// Set when the call ends, which stops the device thread and every
    /// callback still running on it.
    stopped: Arc<AtomicBool>,
    /// While deafened, incoming audio is dropped rather than played.
    deafened: Arc<AtomicBool>,
    /// The sample rate and channel count the capture ring is filled at, which
    /// the mixer is told so it can resample. Falls back to the call's own
    /// format when there's no microphone to ask.
    capture_format: (u32, u32),
}

impl AudioIo {
    /// Opens the microphone and the speakers. `input_device` names the
    /// microphone to use, as [`input_devices`] reported its id; `None`, or one
    /// that isn't plugged in any more, falls back to the system default.
    ///
    /// Either device may fail to open — no microphone, no output device, a
    /// driver that refuses the format — and the call carries on with the half
    /// that worked.
    pub fn start(input_device: Option<&str>) -> Self {
        let host = cpal::default_host();
        // The devices are picked here, on the caller's thread, because the
        // capture format has to be known before the mixer is handed the
        // microphone. Only the streams themselves are `!Send`.
        let input = chosen_input(&host, input_device).and_then(input_config);
        let output = host.default_output_device().and_then(output_config);

        let this = Self {
            capture: Arc::default(),
            playback: Arc::default(),
            stopped: Arc::new(AtomicBool::new(false)),
            deafened: Arc::new(AtomicBool::new(false)),
            capture_format: input
                .as_ref()
                .map_or((SAMPLE_RATE, CHANNELS as u32), |(_, config)| {
                    (
                        config.sample_rate(),
                        ring_channels(config.channels()) as u32,
                    )
                }),
        };

        let capture = this.capture.clone();
        let playback = this.playback.clone();
        let stopped = this.stopped.clone();
        std::thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || run_devices(input, output, &capture, &playback, &stopped))
            .ok();

        this
    }

    /// The microphone as something songbird can play: raw PCM in the device's
    /// own format that never ends. The mixer resamples it to the call's.
    pub fn mic_input(&self) -> Input {
        let (sample_rate, channels) = self.capture_format;
        RawAdapter::new(
            MicSource {
                capture: self.capture.clone(),
                stopped: self.stopped.clone(),
            },
            sample_rate,
            channels,
        )
        .into()
    }

    /// Queues decoded audio from the call for playback. Songbird hands over
    /// 20ms of interleaved stereo `i16` per tick.
    pub fn play(&self, samples: &[i16]) {
        if self.deafened.load(Ordering::Relaxed) || self.stopped.load(Ordering::Relaxed) {
            return;
        }
        self.playback
            .push(samples.iter().map(|&sample| f32::from(sample) / 32768.));
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.deafened.store(deafened, Ordering::Relaxed);
        if deafened {
            // Drop what's already queued, so undeafening doesn't replay a
            // burst of stale audio.
            self.playback.clear();
        }
    }

    /// Closes both devices and ends the thread holding them open.
    ///
    /// Called outright rather than left to [`Drop`], because songbird's event
    /// handlers hold the same `Arc` and outlive the call by however long the
    /// driver takes to shut down — long enough to leave the microphone light
    /// on after the user hung up.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.capture.clear();
        self.playback.clear();
    }
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The mixer's view of the microphone: a stream of raw little-endian `f32`
/// that returns silence rather than ending when the device falls behind.
///
/// Ending it would end the track, and the call would go quiet for good.
struct MicSource {
    capture: Arc<SampleRing>,
    stopped: Arc<AtomicBool>,
}

impl Read for MicSource {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.stopped.load(Ordering::Relaxed) {
            return Ok(0);
        }

        let count = buf.len() / size_of::<f32>();
        if count == 0 {
            return Ok(0);
        }

        let mut samples = vec![0f32; count];
        self.capture.fill(&mut samples);
        for (slot, sample) in buf.chunks_exact_mut(size_of::<f32>()).zip(samples) {
            slot.copy_from_slice(&sample.to_le_bytes());
        }

        Ok(count * size_of::<f32>())
    }
}

impl Seek for MicSource {
    fn seek(&mut self, _pos: SeekFrom) -> IoResult<u64> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

impl MediaSource for MicSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// A microphone the user can pick between, as the menu lists it.
pub struct InputDevice {
    /// Stable across runs and reboots, so it's what a preference stores.
    pub id: String,
    pub name: String,
}

/// Every microphone the system currently offers.
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
fn chosen_input(host: &cpal::Host, id: Option<&str>) -> Option<cpal::Device> {
    id.and_then(|id| id.parse::<DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_input_device())
}

/// A microphone and the configuration to open it with.
fn input_config(device: cpal::Device) -> Option<(cpal::Device, SupportedStreamConfig)> {
    let supported = device.default_input_config().ok()?;
    Some((device, supported))
}

/// The same for the speakers.
fn output_config(device: cpal::Device) -> Option<(cpal::Device, SupportedStreamConfig)> {
    let supported = device.default_output_config().ok()?;
    Some((device, supported))
}

/// How many of a microphone's channels the capture ring carries.
///
/// The raw input the mixer is handed can only describe mono or stereo, so a
/// device offering more — a mic array, a capture interface — is read as the
/// stereo pair it starts with rather than refused outright.
fn ring_channels(device_channels: u16) -> usize {
    usize::from(device_channels).clamp(1, CHANNELS)
}

/// Opens a stream in whichever sample format the device reports.
///
/// `build` names a function generic over the sample type, and the arguments
/// after it are handed to it unchanged. The formats left out are the ones no
/// host offers as a device default.
macro_rules! in_device_format {
    ($format:expr, $build:ident $(, $arg:expr)* $(,)?) => {
        match $format {
            SampleFormat::F32 => $build::<f32>($($arg),*),
            SampleFormat::F64 => $build::<f64>($($arg),*),
            SampleFormat::I8 => $build::<i8>($($arg),*),
            SampleFormat::I16 => $build::<i16>($($arg),*),
            SampleFormat::I32 => $build::<i32>($($arg),*),
            SampleFormat::U8 => $build::<u8>($($arg),*),
            SampleFormat::U16 => $build::<u16>($($arg),*),
            SampleFormat::U32 => $build::<u32>($($arg),*),
            _ => None,
        }
    };
}

/// Opens both streams and holds them until the call ends. Runs on its own
/// thread: cpal streams are `!Send` on some platforms, so they can't be handed
/// back to the caller.
fn run_devices(
    input: Option<(cpal::Device, SupportedStreamConfig)>,
    output: Option<(cpal::Device, SupportedStreamConfig)>,
    capture: &Arc<SampleRing>,
    playback: &Arc<SampleRing>,
    stopped: &Arc<AtomicBool>,
) {
    let input = input.and_then(|(device, config)| build_input(&device, config, capture.clone()));
    let output =
        output.and_then(|(device, config)| build_output(&device, config, playback.clone()));

    if input.is_none() && output.is_none() {
        return;
    }

    // Both streams stop when they're dropped, so the thread's only job from
    // here is to stay alive for as long as the call does.
    while !stopped.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Fills the capture ring with what the device hears, as `f32` at the device's
/// own rate; the mixer is told that rate and does the resampling.
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
            |_| {},
            None,
        )
        .ok()
}

/// Copies one callback's worth of microphone into the ring, keeping only the
/// channels [`ring_channels`] promised the mixer.
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
    playback: Arc<SampleRing>,
) -> Option<cpal::Stream> {
    let stream = in_device_format!(
        config.sample_format(),
        output_stream,
        device,
        config.into(),
        playback,
    )?;
    stream.play().ok()?;
    Some(stream)
}

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    playback: Arc<SampleRing>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &_| {
                let frames = data.len() / channels.max(1);
                // Take the call's own 48kHz stereo, then lay it out in the
                // device's rate and channel count.
                let wanted = (frames as u64 * u64::from(SAMPLE_RATE) / u64::from(rate.max(1)))
                    as usize
                    * CHANNELS;
                let mut source = vec![0f32; wanted];
                playback.fill(&mut source);
                from_call_format(&source, data, channels);
            },
            |_| {},
            None,
        )
        .ok()
}

/// The call's format to the device's: `source` is 48kHz stereo, `out` is
/// whatever the output stream asked for — the caller sized `source` for the
/// device's rate, so the frame counts are all this needs.
fn from_call_format<T>(source: &[f32], out: &mut [T], channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    if channels == 0 {
        return;
    }
    let in_frames = source.len() / CHANNELS;
    let out_frames = out.len() / channels;
    if in_frames == 0 {
        out.fill(T::EQUILIBRIUM);
        return;
    }

    for frame in 0..out_frames {
        let source_frame = source_frame(frame, out_frames, in_frames);
        let left = source[source_frame * CHANNELS];
        let right = source[source_frame * CHANNELS + 1];

        for (channel, slot) in out[frame * channels..][..channels].iter_mut().enumerate() {
            *slot = T::from_sample(match channel {
                0 => left,
                1 => right,
                // Anything past stereo (a surround device) gets the mix, which
                // is better than leaving those speakers silent.
                _ => (left + right) / 2.,
            });
        }
    }

    let filled = out_frames * channels;
    out[filled..].fill(T::EQUILIBRIUM);
}

/// The input frame that best lines up with output frame `frame`.
///
/// Nearest-neighbour is enough for speech coming out of a call, and costs
/// nothing in an audio callback.
fn source_frame(frame: usize, out_frames: usize, in_frames: usize) -> usize {
    if out_frames == 0 || in_frames == 0 {
        return 0;
    }
    (frame * in_frames / out_frames).min(in_frames - 1)
}
