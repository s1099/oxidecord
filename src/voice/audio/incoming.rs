//! Decoding one sender's packets, and concealing the ones that were lost.

use std::sync::Arc;

use opus2::Decoder;
use songbird::constants::{MONO_FRAME_SIZE, SAMPLE_RATE};

use super::CHANNELS;
use super::ring::SampleRing;

/// Opus's longest frame (120ms), per channel.
const MAX_FRAME: usize = 5760;

/// Without this buffer, network jitter makes nearly every callback underrun, and
/// gaps repeating at the callback rate sound like a buzz instead of dropouts.
const PLAYBACK_PRIME_MS: u32 = 60;

/// A longer run is a sender that dropped out; concealing it would only delay
/// the audio that follows.
const MAX_CONCEALED: i16 = 5;

/// Further behind than this is a sender that restarted its count, not a late packet.
const REORDER_WINDOW: i16 = 32;

pub(super) struct Incoming {
    decoder: Decoder,
    pub(super) ring: Arc<SampleRing>,
    expected: Option<u16>,
    /// Samples per channel of the last frame; sizes a concealed frame.
    frame: usize,
    decoded: Vec<f32>,
}

impl Incoming {
    pub(super) fn new() -> Option<Self> {
        Some(Self {
            decoder: Decoder::new(SAMPLE_RATE, opus2::Channels::Stereo).ok()?,
            ring: Arc::new(SampleRing::new(SAMPLE_RATE, CHANNELS, PLAYBACK_PRIME_MS)),
            expected: None,
            frame: MONO_FRAME_SIZE,
            decoded: vec![0.; MAX_FRAME * CHANNELS],
        })
    }

    /// Plays in arrival order rather than sorting: reordering is rare enough
    /// that waiting would cost every packet latency. Late packets were already
    /// concealed, so they're dropped.
    pub(super) fn receive(&mut self, sequence: u16, opus: &[u8]) {
        let gap = self
            .expected
            .map_or(0, |expected| sequence.wrapping_sub(expected) as i16);
        if (-REORDER_WINDOW..0).contains(&gap) {
            return;
        }
        self.expected = Some(sequence.wrapping_add(1));

        if (1..=MAX_CONCEALED).contains(&gap) {
            for _ in 1..gap {
                self.decode(&[], false);
            }
            // With FEC this recovers the previous frame from this packet; without, Opus conceals.
            self.decode(opus, true);
        }
        self.decode(opus, false);
    }

    fn decode(&mut self, opus: &[u8], fec: bool) {
        // Concealment and FEC fill exactly the room given, so it must be one frame.
        let room = if opus.is_empty() || fec {
            self.frame * CHANNELS
        } else {
            self.decoded.len()
        };
        let Ok(frames) = self
            .decoder
            .decode_float(opus, &mut self.decoded[..room], fec)
        else {
            return;
        };
        if !opus.is_empty() && !fec {
            self.frame = frames;
        }
        self.ring
            .push(self.decoded[..frames * CHANNELS].iter().copied());
    }
}
