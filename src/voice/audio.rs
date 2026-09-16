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
//! Neither side of a ring shares a clock with the other, so a ring is either
//! running dry or running over most of the time. The rings answer for that
//! rather than the callbacks: see [`SampleRing`] for the priming and the ramps
//! that keep a stall from being heard, and [`Playback`] for the phase the
//! resampler has to carry between callbacks.
//!
//! cpal streams are not `Send` on every platform, so both live on a thread of
//! their own that does nothing but hold them open until the call ends.

use std::collections::VecDeque;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{
    DeviceId, FromSample, Sample as _, SampleFormat, SizedSample, StreamConfig,
    SupportedStreamConfig,
};
use songbird::constants::SAMPLE_RATE;
use songbird::input::core::io::MediaSource;
use songbird::input::{Input, RawAdapter};

/// Decoded packets are stereo, which is the layout the playback ring carries.
const CHANNELS: usize = 2;

/// Ceiling on each ring: enough to ride out a scheduling hiccup, short enough
/// that recovery isn't audible as a growing delay.
const MAX_BUFFERED_MS: u32 = 400;

/// How much the playback ring builds up before it starts draining again.
///
/// Packets arrive on a 20ms tick that the network smears either side of, so a
/// reader that drains to empty underruns on nearly every callback. The gaps
/// that leaves repeat at the callback rate, which is heard as a harsh buzz
/// over the speech rather than as the dropouts they are; a buffer this deep
/// costs latency once and stops it.
const PLAYBACK_PRIME_MS: u32 = 40;

/// The same for the microphone, which only has device jitter to absorb rather
/// than a network as well.
const CAPTURE_PRIME_MS: u32 = 30;

/// How long the ramp either side of a gap lasts.
///
/// Long enough to take the edge off the step, short enough not to swallow a
/// consonant.
const FADE_MS: u32 = 3;

/// A ring of interleaved samples shared between an audio callback and the
/// call.
///
/// Neither side ever blocks: a starved reader gets silence and an overrun
/// writer drops the oldest audio, since stalling either thread would be heard
/// as a stutter across the whole call.
///
/// Both of those are steps in the waveform, and a step is a click — the louder
/// the audio it interrupts, the louder the click. So the ring hides them: it
/// ramps down into a gap and back up out of one, and after a stall it waits
/// for `prime` samples to build up rather than tearing another gap on the very
/// next callback.
struct SampleRing {
    state: Mutex<RingState>,
    /// Interleaved channel count. Every drain and every drop is a whole number
    /// of frames, because losing a single sample would rotate every later one
    /// onto the wrong channel for the rest of the call.
    channels: usize,
    /// Samples the reader waits for after a stall before it drains again.
    prime: usize,
    capacity: usize,
    /// Length of the ramp either side of a gap, in frames.
    fade: usize,
}

#[derive(Default)]
struct RingState {
    samples: VecDeque<f32>,
    /// While set the reader emits silence: the writer hasn't built the buffer
    /// back up since the last stall.
    priming: bool,
    /// The last sample emitted on each channel. A gap ramps down from here
    /// rather than cutting straight to zero.
    last: Vec<f32>,
    /// Frames left of the ramp back in after a gap.
    fade_in: usize,
}

impl SampleRing {
    fn new(sample_rate: u32, channels: usize, prime_ms: u32) -> Self {
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
            // A ring that couldn't hold what the reader waits for would never
            // finish priming.
            capacity: (frames(MAX_BUFFERED_MS) * channels).max(prime * 2),
            fade: frames(FADE_MS).max(1),
        }
    }

    fn push(&self, samples: impl Iterator<Item = f32>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        // A driver that hands back a NaN, or a mix that ran past full scale,
        // becomes a full-scale spike once it's converted to the device's
        // format. Neither belongs in a call, and neither is worth finding out
        // about through the speakers.
        state.samples.extend(samples.map(sanitize));

        if state.samples.len() > self.capacity {
            // Keep the newest audio: the listener would rather skip forward
            // than fall further behind the conversation.
            let excess = state.samples.len() - self.capacity;
            let excess = (excess.div_ceil(self.channels) * self.channels).min(state.samples.len());
            state.samples.drain(..excess);
            // The reader is about to jump over whatever was dropped, so ramp
            // it back in on the far side of the jump.
            state.fade_in = self.fade;
        }
    }

    /// Fills `out` with what's buffered, ramping into silence when the writer
    /// hasn't kept up. `out` holds a whole number of frames.
    fn fill(&self, out: &mut [f32]) {
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
            // Underrun. Ease out of the audio there was and wait for the
            // writer to get ahead again, rather than cutting it dead here and
            // tearing another gap on the next callback.
            self.fade_out(&mut out[..take]);
            out[take..].fill(0.);
            state.priming = true;
        }

        if state.fade_in > 0 {
            self.fade_in(out, &mut state.fade_in);
        }

        // Remember where the waveform ended, so the next gap knows what to
        // ramp down from.
        let frames = out.len() / self.channels;
        let last_frame = &out[(frames - 1) * self.channels..][..self.channels];
        state.last.copy_from_slice(last_frame);
    }

    /// Ramps `out` down from the last sample emitted, then holds silence.
    ///
    /// Used when there is nothing at all to play: cutting from the previous
    /// callback's final sample straight to zero is the click.
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

    /// Ramps the end of what was filled down to silence, so the gap that
    /// follows isn't a step.
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

    /// Ramps the start of `out` up out of a gap, carrying on across callbacks
    /// when the ramp outlasts one of them.
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

    fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.samples.clear();
            state.priming = true;
            state.fade_in = 0;
            state.last.fill(0.);
        }
    }
}

/// Keeps a sample inside the range the device conversions assume.
///
/// A NaN compares false against every bound, so it has to be caught before the
/// clamp rather than by it.
fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1., 1.)
    } else {
        0.
    }
}

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

/// Ends the call for the device thread and everything it holds open.
///
/// The flag is what the callbacks read, on the audio threads, where taking a
/// lock is the one thing that must not happen. The condition variable is how
/// the device thread hears about it: polling for the flag left the microphone
/// light on for up to a poll interval after the user hung up.
#[derive(Default)]
struct Shutdown {
    stopped: AtomicBool,
    guard: Mutex<()>,
    signal: Condvar,
}

impl Shutdown {
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        // Taken and dropped purely to pair with the waiter: without it a stop
        // landing between the waiter's check and its wait would be missed.
        let _guard = self.guard.lock();
        self.signal.notify_all();
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Blocks until [`Shutdown::stop`] is called.
    fn wait(&self) {
        let Ok(mut guard) = self.guard.lock() else {
            return;
        };
        while !self.is_stopped() {
            let Ok(next) = self.signal.wait(guard) else {
                return;
            };
            guard = next;
        }
    }
}

/// The audio devices for one call.
pub struct AudioIo {
    capture: Arc<SampleRing>,
    playback: Arc<SampleRing>,
    /// Set when the call ends, which stops the device thread and every
    /// callback still running on it.
    shutdown: Arc<Shutdown>,
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
        // microphone — and before the capture ring can be sized in it. Only
        // the streams themselves are `!Send`.
        let input = chosen_input(&host, input_device).and_then(input_config);
        let output = host.default_output_device().and_then(output_config);

        let capture_format =
            input
                .as_ref()
                .map_or((SAMPLE_RATE, CHANNELS as u32), |(_, config)| {
                    (
                        config.sample_rate(),
                        ring_channels(config.channels()) as u32,
                    )
                });

        let this = Self {
            capture: Arc::new(SampleRing::new(
                capture_format.0,
                capture_format.1 as usize,
                CAPTURE_PRIME_MS,
            )),
            playback: Arc::new(SampleRing::new(SAMPLE_RATE, CHANNELS, PLAYBACK_PRIME_MS)),
            shutdown: Arc::default(),
            deafened: Arc::new(AtomicBool::new(false)),
            capture_format,
        };

        let capture = this.capture.clone();
        let playback = this.playback.clone();
        let shutdown = this.shutdown.clone();
        std::thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || run_devices(input, output, &capture, &playback, &shutdown))
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
                shutdown: self.shutdown.clone(),
            },
            sample_rate,
            channels,
        )
        .into()
    }

    /// Queues decoded audio from the call for playback. Songbird hands over
    /// 20ms of interleaved stereo per tick, summed across everyone audible —
    /// which is why this takes a wider sample than it stores.
    pub fn play(&self, samples: &[i32]) {
        if self.deafened.load(Ordering::Relaxed) || self.shutdown.is_stopped() {
            return;
        }
        self.playback
            .push(samples.iter().map(|&sample| limit(sample as f32 / 32768.)));
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
        self.shutdown.stop();
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
    shutdown: Arc<Shutdown>,
}

impl Read for MicSource {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.shutdown.is_stopped() {
            return Ok(0);
        }

        // Whole frames only: handing back a partial one would put every later
        // sample on the wrong channel. A buffer too small to hold one is the
        // only case with nothing to hand back, and `Ok(0)` there would read as
        // the end of the track.
        let channels = self.capture.channels;
        let count = (buf.len() / size_of::<f32>() / channels) * channels;
        if count == 0 {
            return Err(std::io::ErrorKind::Interrupted.into());
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
    shutdown: &Arc<Shutdown>,
) {
    let input = input.and_then(|(device, config)| build_input(&device, config, capture.clone()));
    let output =
        output.and_then(|(device, config)| build_output(&device, config, playback.clone()));

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
            // Usually the device going away mid-call, which is worth knowing
            // about when a call has gone one-way.
            |err| eprintln!("microphone stream error: {err}"),
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
    let mut render = Playback::new(playback, config.sample_rate, usize::from(config.channels));

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &_| render.render(data),
            |err| eprintln!("speaker stream error: {err}"),
            None,
        )
        .ok()
}

/// Lays the call's 48kHz stereo out in the output device's rate and channel
/// count.
///
/// The position between source frames is carried across callbacks, and so are
/// the two frames it sits between. Resampling each callback from scratch put a
/// step in the waveform at every buffer boundary, and a step that repeats at
/// the callback rate is heard as a tone over the speech rather than as the
/// glitches it is.
struct Playback {
    ring: Arc<SampleRing>,
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
    channels: usize,
}

impl Playback {
    fn new(ring: Arc<SampleRing>, sample_rate: u32, channels: usize) -> Self {
        Self {
            ring,
            previous: [0.; CHANNELS],
            next: [0.; CHANNELS],
            phase: 0.,
            step: f64::from(SAMPLE_RATE) / f64::from(sample_rate.max(1)),
            scratch: Vec::new(),
            channels: channels.max(1),
        }
    }

    fn render<T>(&mut self, out: &mut [T])
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
        self.ring.fill(&mut self.scratch);

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
