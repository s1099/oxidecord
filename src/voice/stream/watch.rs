//! Watching someone else's stream.
//!
//! The viewer's side of [`super`]: the same kind of connection, to the same
//! server the streamer sends to, but receiving. The server forwards the
//! streamer's RTP; this reorders it, asks for what was lost, rebuilds each
//! frame, decrypts it with DAVE, and hands it to the platform decoder.
//!
//! Nothing can be decoded until a keyframe, and nothing after a lost frame
//! decodes cleanly until the next, so frames are skipped until one arrives
//! and the streamer is asked for one (a picture loss indication) whenever
//! that's the state.

use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use twilight_model::id::{Id, marker::UserMarker};

use crate::platform::h264::{self, DecoderEvent, DecoderInput, LiveDecoder};
use crate::platform::runtime;

use super::media::{H264_PAYLOAD, Media, RTX_PAYLOAD, RtpPacket};
use super::receive::{Assembled, Assembler};
use super::signalling::{Signal, Signalling};
use super::{StreamConnection, StreamEvent};

/// How often held packets are checked for gaps that have waited too long.
const ASSEMBLE_INTERVAL: Duration = Duration::from_millis(20);

/// The least time between keyframe requests. The sender needs a moment to
/// produce one, and asking again meanwhile only makes it produce two.
const KEYFRAME_RETRY: Duration = Duration::from_millis(500);

/// How often a receiver report goes out to keep the path open.
const REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// One stream being watched. Dropping it closes the connection and stops the
/// decoder.
pub struct WatchStream {
    _stop: oneshot::Sender<()>,
    decoder: LiveDecoder,
}

impl WatchStream {
    /// Connects to `streamer`'s stream. Decoded pictures arrive on `frames`,
    /// scaled to fit `target`; the connection's end arrives on `events`.
    pub fn start(
        connection: StreamConnection,
        streamer: Id<UserMarker>,
        target: (u32, u32),
        events: UnboundedSender<StreamEvent>,
        frames: UnboundedSender<DecoderEvent>,
    ) -> Self {
        let (decoder, input) = h264::start(target, frames);
        let (stop, stopped) = oneshot::channel();
        runtime::handle().spawn(async move {
            let stream_key = connection.stream_key.clone();
            let error = match run(connection, streamer.get(), input, stopped).await {
                Ok(Outcome::Stopped) => return,
                Ok(Outcome::Ended) => None,
                Err(err) => Some(err),
            };
            let _ = events.unbounded_send(StreamEvent::Ended { stream_key, error });
        });
        Self {
            _stop: stop,
            decoder,
        }
    }

    /// Changes the size pictures are scaled to fit, in physical pixels.
    pub fn set_target(&self, target: (u32, u32)) {
        self.decoder.set_target(target);
    }

    /// Reports that a picture has been drawn and released. See
    /// [`LiveDecoder::frame_consumed`].
    pub fn frame_consumed(&self) {
        self.decoder.frame_consumed();
    }
}

enum Outcome {
    /// Stopped by the app, or the decoder went away.
    Stopped,
    /// The stream ended.
    Ended,
}

async fn run(
    connection: StreamConnection,
    streamer: u64,
    decoder: DecoderInput,
    mut stopped: oneshot::Receiver<()>,
) -> Result<Outcome, String> {
    let (mut signalling, ready) = Signalling::connect(&connection).await?;
    let (mut media, external) = Media::connect(ready.ip, ready.port, ready.ssrcs).await?;
    let (mode, key) = signalling.select_protocol(external, ready.mode).await?;
    media.set_key(&mode, &key)?;
    signalling.announce_watching(ready.ssrcs).await?;

    let mut heartbeat = tokio::time::interval(signalling.heartbeat);
    let mut assemble = tokio::time::interval(ASSEMBLE_INTERVAL);
    let mut reports = tokio::time::interval(REPORT_INTERVAL);
    let mut buffer = vec![0u8; 2048];
    let mut viewer = Viewer {
        streamer,
        assembler: Assembler::default(),
        guessed_ssrc: None,
        needs_keyframe: true,
        last_request: None,
    };

    let outcome = loop {
        let assembled = tokio::select! {
            _ = &mut stopped => break Outcome::Stopped,
            _ = heartbeat.tick() => {
                signalling.send_heartbeat().await?;
                continue;
            }
            _ = reports.tick() => {
                media.send_receiver_report().await;
                continue;
            }
            signal = signalling.next() => match signal? {
                Signal::Nothing => continue,
                Signal::Closed(None) => break Outcome::Ended,
                Signal::Closed(Some(err)) => return Err(err),
            },
            _ = assemble.tick() => viewer.assembler.assemble(Instant::now()),
            received = media.recv(&mut buffer) => {
                let Ok(length) = received else { continue };
                let Some(packet) = media.open_rtp(&buffer[..length]) else { continue };
                viewer.receive(packet, &signalling);
                viewer.assembler.assemble(Instant::now())
            }
        };

        let Some(ssrc) = viewer.video_ssrc(&signalling) else {
            continue;
        };
        if !assembled.nack.is_empty() {
            media.send_nack(ssrc, &assembled.nack).await;
        }
        if !viewer.decode(assembled, &mut signalling, &decoder) {
            break Outcome::Stopped;
        }
        if viewer.wants_keyframe() {
            media.send_picture_loss(ssrc).await;
        }
    };

    signalling.close().await;
    Ok(outcome)
}

/// What the receiving loop keeps between packets.
struct Viewer {
    streamer: u64,
    assembler: Assembler,
    /// The video SSRC taken from the first H.264 packet, for a server that
    /// forwards video before it says whose it is.
    guessed_ssrc: Option<u32>,
    /// Nothing decodable has arrived since the start or the last loss.
    needs_keyframe: bool,
    last_request: Option<Instant>,
}

impl Viewer {
    fn video_ssrc(&self, signalling: &Signalling) -> Option<u32> {
        signalling
            .remote
            .map(|remote| remote.ssrc)
            .or(self.guessed_ssrc)
    }

    /// Files a packet with the assembler: the streamer's video as is, and a
    /// retransmission under the sequence number it's standing in for.
    fn receive(&mut self, packet: RtpPacket, signalling: &Signalling) {
        let now = Instant::now();
        let remote = signalling.remote;
        let video = self.video_ssrc(signalling);
        let rtx = remote.and_then(|remote| remote.rtx_ssrc);

        if Some(packet.ssrc) == video {
            self.assembler.insert(
                packet.sequence,
                packet.timestamp,
                packet.marker,
                packet.payload,
                now,
            );
        } else if Some(packet.ssrc) == rtx
            || (packet.payload_type == RTX_PAYLOAD && video.is_some())
        {
            // RFC 4588: the original sequence number, then the payload.
            if let [high, low, payload @ ..] = packet.payload.as_slice() {
                self.assembler.insert(
                    u16::from_be_bytes([*high, *low]),
                    packet.timestamp,
                    packet.marker,
                    payload.to_vec(),
                    now,
                );
            }
        } else if video.is_none() && packet.payload_type == H264_PAYLOAD {
            self.guessed_ssrc = Some(packet.ssrc);
            self.assembler.insert(
                packet.sequence,
                packet.timestamp,
                packet.marker,
                packet.payload,
                now,
            );
        }
    }

    /// Decrypts and decodes what came out of the assembler, skipping what
    /// can't be decoded yet. Returns false once the decoder is gone.
    fn decode(
        &mut self,
        assembled: Assembled,
        signalling: &mut Signalling,
        decoder: &DecoderInput,
    ) -> bool {
        if assembled.lost {
            self.needs_keyframe = true;
        }
        for frame in assembled.frames {
            if self.needs_keyframe && !frame.keyframe {
                continue;
            }
            let Some(plain) = signalling.dave.unprotect(self.streamer, frame.data) else {
                // A frame that won't decrypt leaves the decoder without its
                // reference; the next keyframe after the group forms is
                // where to start.
                self.needs_keyframe = true;
                continue;
            };
            self.needs_keyframe = false;
            if !decoder.decode(plain) {
                return false;
            }
        }
        true
    }

    /// Whether to ask for a keyframe now, noting the request if so.
    fn wants_keyframe(&mut self) -> bool {
        let now = Instant::now();
        if !self.needs_keyframe
            || self
                .last_request
                .is_some_and(|last| now.duration_since(last) < KEYFRAME_RETRY)
        {
            return false;
        }
        self.last_request = Some(now);
        true
    }
}
