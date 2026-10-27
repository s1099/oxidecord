//! The microphone and the speakers.
//!
//! Rings of interleaved `f32` sit between the audio devices and the call:
//! one for capture, which the mixer drains through [`MicSource`], and one per
//! sender for playback, which [`Incoming`] fills from the packets as they
//! arrive and the output device drains through [`Mixer`].
//!
//! Capture stays at the device's own sample rate — songbird's mixer resamples
//! whatever an input declares, so there's nothing to convert on the way in.
//! Playback is the other way round: packets are decoded to 48kHz stereo, and
//! the output stream lays them out for the device.
//!
//! Packets are decoded here rather than by songbird. Its decoder hands Opus
//! the DAVE trailer along with the frame on an end-to-end encrypted call,
//! which SILK mostly shrugs off but CELT — music, and anything else sent at
//! full band — reads its raw bits from the end of, and falls apart on. Its
//! playout also holds packets back by RTP timestamp, which turns a sender
//! whose timestamps run ahead of its packets into stretches of silence.
//!
//! Both directions do convert sample formats, to `f32` and back: the format a
//! device reports is whichever one its driver prefers, and a call with no
//! microphone is worse than a handful of conversions.
//!
//! Neither side of a ring shares a clock with the other, so a ring is either
//! running dry or running over most of the time. The rings answer for that
//! rather than the callbacks: see [`SampleRing`](ring::SampleRing) for the
//! priming and the ramps that keep a stall from being heard, and
//! [`Playback`](playback::Playback) for the phase the resampler has to carry
//! between callbacks.
//!
//! cpal streams are not `Send` on every platform, so both live on a thread of
//! their own that does nothing but hold them open until the call ends.

mod devices;
mod incoming;
mod playback;
mod ring;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use songbird::constants::SAMPLE_RATE;
use songbird::input::core::io::MediaSource;
use songbird::input::{Input, RawAdapter};

use devices::{chosen_input, input_config, output_config, ring_channels, run_devices};
pub use devices::{input_devices, output_devices};
use incoming::Incoming;
use playback::Mixer;
use ring::SampleRing;

use crate::platform::audio::chosen_output;

/// Whose packets a playback ring holds.
///
/// A watched stream is a connection of its own, and its server hands out
/// SSRCs from its own count, so a stream's can match someone's in the call.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sender {
    Call(u32),
    Stream(u32),
}

/// Packets are decoded to stereo, which is the layout the playback rings carry.
const CHANNELS: usize = 2;

/// How much the capture ring builds up before it starts draining again.
/// Shallower than a sender's ring (`incoming::PLAYBACK_PRIME_MS`): the
/// microphone only has device jitter to absorb rather than a network as well.
const CAPTURE_PRIME_MS: u32 = 30;

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

pub struct AudioIo {
    capture: Arc<SampleRing>,
    /// What the speakers play: every sender's ring.
    mixer: Arc<Mixer>,
    /// Every sender's decoder. Only the call's event handlers and a watched
    /// stream touch these, so they're kept apart from the rings the audio
    /// callback locks.
    incoming: Mutex<HashMap<Sender, Incoming>>,
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
    /// Opens the microphone and the speakers. `input_device` and
    /// `output_device` name them as [`input_devices`] and [`output_devices`]
    /// reported their ids; `None`, or one that isn't plugged in any more,
    /// falls back to the system default.
    ///
    /// Either device may fail to open — no microphone, no output device, a
    /// driver that refuses the format — and the call carries on with the half
    /// that worked.
    pub fn start(input_device: Option<&str>, output_device: Option<&str>) -> Self {
        let host = cpal::default_host();
        // The devices are picked here, on the caller's thread, because the
        // capture format has to be known before the mixer is handed the
        // microphone — and before the capture ring can be sized in it. Only
        // the streams themselves are `!Send`.
        let input = chosen_input(&host, input_device).and_then(input_config);
        let output = chosen_output(&host, output_device).and_then(output_config);

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
            mixer: Arc::default(),
            incoming: Mutex::default(),
            shutdown: Arc::default(),
            deafened: Arc::new(AtomicBool::new(false)),
            capture_format,
        };

        let capture = this.capture.clone();
        let mixer = this.mixer.clone();
        let shutdown = this.shutdown.clone();
        std::thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || run_devices(input, output, &capture, &mixer, &shutdown))
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

    /// Queues one Opus packet, from the call or a watched stream, for
    /// playback. `opus` is the frame alone, with every layer of encryption
    /// already taken off.
    pub fn receive(&self, sender: Sender, sequence: u16, opus: &[u8]) {
        if self.deafened.load(Ordering::Relaxed) || self.shutdown.is_stopped() {
            return;
        }
        let Ok(mut incoming) = self.incoming.lock() else {
            return;
        };
        let stream = match incoming.entry(sender) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let Some(stream) = Incoming::new() else {
                    return;
                };
                self.mixer.add(sender, stream.ring.clone());
                entry.insert(stream)
            }
        };
        stream.receive(sequence, opus);
    }

    /// Stops playing a sender who has left the call, or a stream no longer
    /// watched.
    pub fn forget(&self, sender: Sender) {
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.remove(&sender);
        }
        self.mixer.remove(sender);
    }

    /// Stops playing everyone. Whoever is still talking starts over with a
    /// fresh decoder at their next packet.
    fn forget_all(&self) {
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.clear();
        }
        self.mixer.clear();
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.deafened.store(deafened, Ordering::Relaxed);
        if deafened {
            // Drop what's already queued, so undeafening doesn't replay a
            // burst of stale audio.
            self.forget_all();
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
        self.forget_all();
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
