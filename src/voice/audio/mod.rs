//! The microphone and the speakers.
//!
//! Interleaved `f32` rings sit between the devices and the call: one for capture
//! (drained by the mixer through [`MicSource`]) and one per sender for playback
//! ([`Incoming`] fills, [`Mixer`] drains).
//!
//! Capture stays at the device's rate since songbird's mixer resamples inputs;
//! playback is decoded to 48kHz stereo and laid out for the device.
//!
//! Packets are decoded here, not by songbird: on an E2EE call its decoder passes
//! Opus the DAVE trailer, which CELT (full band, music) reads from the end of
//! and breaks on. Its playout also gates on RTP timestamp, which turns senders
//! whose timestamps run ahead of their packets into silence.
//!
//! The two ends of a ring share no clock, so see [`SampleRing`](ring::SampleRing)
//! for the priming and ramps that hide stalls.
//!
//! cpal streams aren't `Send` everywhere, so both live on a thread that just
//! holds them open.

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

/// A watched stream is its own connection with its own SSRC count, so its SSRCs
/// can collide with the call's.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sender {
    Call(u32),
    Stream(u32),
}

const CHANNELS: usize = 2;

/// Shallower than `incoming::PLAYBACK_PRIME_MS`: capture has no network jitter.
const CAPTURE_PRIME_MS: u32 = 30;

/// The flag is read by audio callbacks, which must not lock. The condvar wakes
/// the device thread; polling left the mic light on after hang-up.
#[derive(Default)]
struct Shutdown {
    stopped: AtomicBool,
    guard: Mutex<()>,
    signal: Condvar,
}

impl Shutdown {
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        // Pairs with the waiter so a stop between its check and wait isn't missed.
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
    mixer: Arc<Mixer>,
    /// Kept apart from the rings the audio callback locks.
    incoming: Mutex<HashMap<Sender, Incoming>>,
    shutdown: Arc<Shutdown>,
    deafened: Arc<AtomicBool>,
    /// (rate, channels) of the capture ring; the call's own format with no mic.
    capture_format: (u32, u32),
}

impl AudioIo {
    /// `None`, or a device no longer plugged in, falls back to the default.
    /// Either device may fail to open; the call carries on with the other.
    pub fn start(input_device: Option<&str>, output_device: Option<&str>) -> Self {
        let host = cpal::default_host();
        // Picked here because the capture format must be known to size the ring
        // and for `mic_input`; only the streams are `!Send`.
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

    /// Endless raw PCM in the device's format; the mixer resamples it.
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

    /// `opus` is the bare frame, all encryption already removed.
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

    pub fn forget(&self, sender: Sender) {
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.remove(&sender);
        }
        self.mixer.remove(sender);
    }

    fn forget_all(&self) {
        if let Ok(mut incoming) = self.incoming.lock() {
            incoming.clear();
        }
        self.mixer.clear();
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.deafened.store(deafened, Ordering::Relaxed);
        if deafened {
            // So undeafening doesn't replay stale audio.
            self.forget_all();
        }
    }

    /// Called explicitly rather than left to [`Drop`]: songbird's handlers hold
    /// the same `Arc` past hang-up, which left the mic light on.
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

/// Raw little-endian `f32`. Returns silence rather than ending when the device
/// falls behind, since ending would end the track for good.
struct MicSource {
    capture: Arc<SampleRing>,
    shutdown: Arc<Shutdown>,
}

impl Read for MicSource {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.shutdown.is_stopped() {
            return Ok(0);
        }

        // Whole frames only, or later samples land on the wrong channel. `Ok(0)`
        // would read as end of track, hence `Interrupted` for a too-small buffer.
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
