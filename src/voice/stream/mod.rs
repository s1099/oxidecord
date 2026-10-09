//! Go Live: streaming the screen into a call.
//!
//! A stream is a second voice connection, separate from the call's. The main
//! gateway is asked to create the stream, answers with a server and token of
//! its own, and the screen goes to that server as H.264 over RTP, end-to-end
//! encrypted with DAVE like the call's audio. Everyone watching connects to
//! the same server; the streamer never hears from them directly, only
//! through the RTCP feedback the server relays.
//!
//! The whole connection — websocket, UDP socket, DAVE session — lives in one
//! task on the shared runtime, so none of it needs a lock. [`GoLive`] is the
//! only handle the app holds, and dropping it ends the stream.

mod media;
mod signalling;
mod sps;

use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use twilight_model::id::{Id, marker::UserMarker};

use crate::platform::capture::{
    CaptureEvent, CaptureSettings, CaptureSource, EncodedFrame, ScreenCapture, nal_units,
};
use crate::platform::runtime;

use media::Media;
use signalling::{Signal, Signalling};

/// How often a sender report goes out, so viewers can keep the stream's
/// clock in step with theirs.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// How often the console gets a summary of the stream.
const SUMMARY_INTERVAL: Duration = Duration::from_secs(5);

/// Everything needed to open a stream connection, gathered from the main
/// gateway's `STREAM_CREATE` and `STREAM_SERVER_UPDATE`.
pub struct StreamConnection {
    pub stream_key: String,
    /// The stream server's id, which the connection identifies to.
    pub server_id: String,
    /// The DAVE group's channel id, derived from [`Self::server_id`].
    pub dave_channel_id: u64,
    pub user_id: Id<UserMarker>,
    /// The call's voice session: a stream belongs to the session that
    /// created it.
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
}

impl StreamConnection {
    /// DAVE's channel id for a stream: the stream server's id, less one.
    pub fn dave_channel_for(server_id: &str) -> Option<u64> {
        server_id.parse::<u64>().ok()?.checked_sub(1)
    }
}

/// What a stream reports back.
pub enum StreamEvent {
    /// The screen is going out.
    Live { stream_key: String },
    /// The stream is over. `None` when it ended normally — the shared window
    /// closed, or the server ended it — and the reason when it failed.
    Ended {
        stream_key: String,
        error: Option<String>,
    },
}

/// One running stream. Dropping it stops the capture and closes the
/// connection.
pub struct GoLive {
    _stop: oneshot::Sender<()>,
}

impl GoLive {
    pub fn start(
        connection: StreamConnection,
        source: CaptureSource,
        settings: CaptureSettings,
        events: UnboundedSender<StreamEvent>,
    ) -> Self {
        let (stop, stopped) = oneshot::channel();
        runtime::handle().spawn(async move {
            let stream_key = connection.stream_key.clone();
            let outcome = run(connection, source, settings, stopped, &events).await;
            let error = match outcome {
                // Stopped by the app, which already knows.
                Ok(Outcome::Stopped) => return,
                Ok(Outcome::Ended) => None,
                Err(err) => Some(err),
            };
            let _ = events.unbounded_send(StreamEvent::Ended { stream_key, error });
        });
        Self { _stop: stop }
    }
}

enum Outcome {
    Stopped,
    Ended,
}

async fn run(
    connection: StreamConnection,
    source: CaptureSource,
    settings: CaptureSettings,
    mut stopped: oneshot::Receiver<()>,
    events: &UnboundedSender<StreamEvent>,
) -> Result<Outcome, String> {
    eprintln!(
        "screen share: connecting to {} (server {}, DAVE group {})",
        connection.endpoint, connection.server_id, connection.dave_channel_id
    );
    let (mut signalling, ready) = Signalling::connect(&connection).await?;
    eprintln!(
        "screen share: ready, media to {}:{}, SSRCs audio {} video {} rtx {}",
        ready.ip, ready.port, ready.ssrcs.audio, ready.ssrcs.video, ready.ssrcs.rtx
    );
    let (mut media, external) = Media::connect(ready.ip, ready.port, ready.ssrcs).await?;
    let (mode, key) = signalling.select_protocol(external, ready.mode).await?;
    media.set_key(&mode, &key)?;

    let (capture_events, mut captured) = unbounded();
    let capture = ScreenCapture::start(source, settings, capture_events);

    let mut heartbeat = tokio::time::interval(signalling.heartbeat);
    let mut reports = tokio::time::interval(REPORT_INTERVAL);
    let mut summaries = tokio::time::interval(SUMMARY_INTERVAL);
    summaries.reset();
    let mut audio = tokio::time::interval(Duration::from_millis(20));
    let send_audio = media.experiments.audio;
    let mut buffer = vec![0u8; 1500];
    // Frames can't go out until DAVE's group has formed; the first one that
    // does has to be a keyframe, or there's nothing for viewers to start on.
    let mut was_ready = false;
    let mut stats = Stats::default();

    let outcome = loop {
        tokio::select! {
            _ = &mut stopped => break Outcome::Stopped,
            _ = heartbeat.tick() => signalling.send_heartbeat().await?,
            _ = reports.tick() => media.send_report().await,
            _ = summaries.tick() => stats.summarize(&media, signalling.dave.is_ready()),
            _ = audio.tick(), if send_audio => {
                // Opus silence, through DAVE once it's up so a viewer's
                // decryptor doesn't choke on it.
                const SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];
                if let Some(frame) = signalling.dave.protect_audio(&SILENCE) {
                    media.send_audio(&frame).await;
                }
            },
            signal = signalling.next() => match signal? {
                Signal::Nothing => {}
                Signal::Closed(None) => break Outcome::Ended,
                Signal::Closed(Some(err)) => return Err(err),
            },
            event = captured.next() => match event {
                Some(CaptureEvent::Started { width, height }) => {
                    eprintln!("screen share: capturing at {width}x{height}, {} fps", settings.fps);
                    signalling
                        .announce_video(ready.ssrcs, (width, height), settings.fps, settings.bitrate)
                        .await?;
                    let _ = events.unbounded_send(StreamEvent::Live {
                        stream_key: connection.stream_key.clone(),
                    });
                }
                Some(CaptureEvent::Frame(frame)) => {
                    stats.captured += 1;
                    stats.keyframes += u64::from(frame.keyframe);
                    let ready_now = signalling.dave.is_ready();
                    if ready_now != was_ready {
                        eprintln!(
                            "screen share: encryption {}",
                            if ready_now { "ready, sending video" } else { "not ready, holding video" },
                        );
                    }
                    if ready_now && !was_ready && !frame.keyframe {
                        capture.request_keyframe();
                    }
                    was_ready = ready_now;
                    if !ready_now {
                        stats.held += 1;
                        continue;
                    }
                    send(&mut signalling, &mut media, frame, &mut stats).await;
                }
                Some(CaptureEvent::SourceClosed) | None => break Outcome::Ended,
                Some(CaptureEvent::Failed(err)) => return Err(err),
            },
            received = media.recv(&mut buffer) => {
                let Ok(length) = received else { continue };
                let feedback = media.feedback(&buffer[..length]);
                stats.note(&feedback);
                if feedback.keyframe {
                    capture.request_keyframe();
                }
                if !feedback.lost.is_empty() {
                    media.retransmit(&feedback.lost).await;
                }
            },
        }
    };

    signalling.close().await;
    Ok(outcome)
}

async fn send(
    signalling: &mut Signalling,
    media: &mut Media,
    frame: EncodedFrame,
    stats: &mut Stats,
) {
    let frame_data = sps::rewrite_frame(&frame.data);
    let Some(protected) = signalling.dave.protect(&frame_data) else {
        stats.failed += 1;
        return;
    };
    media.send_frame(&protected, frame.timestamp).await;
    stats.note_sent(&frame_data, &protected, frame.keyframe);
}

/// What the stream did, for the console: milestones as they first happen,
/// and a summary every few seconds. When a stream reaches no one, this is
/// all there is to go on.
#[derive(Default)]
struct Stats {
    captured: u64,
    keyframes: u64,
    sent: u64,
    keyframes_sent: u64,
    /// Frames dropped because DAVE's group hadn't formed.
    held: u64,
    /// Frames dropped because DAVE wouldn't encrypt them.
    failed: u64,
    first_sent: bool,
    heard: bool,
    undecryptable: bool,
    keyframe_requested: bool,
    nacked: bool,
    keyframe_requests: u64,
    lost_reported: u64,
    kinds: std::collections::BTreeMap<String, u64>,
    /// RTCP kinds already dumped in full.
    dumped: std::collections::HashSet<String>,
    /// The latest report block about each SSRC.
    reports: std::collections::BTreeMap<u32, media::ReceptionReport>,
    /// The counters at the last summary, to report the change since.
    previous: media::Counters,
}

impl Stats {
    fn note(&mut self, feedback: &media::Feedback) {
        let sent = self.sent;
        let first = |seen: &mut bool, happened: bool, what: &str| {
            if happened && !*seen {
                *seen = true;
                eprintln!("screen share: {what} (after {sent} frames sent)");
            }
        };
        first(
            &mut self.heard,
            feedback.heard,
            "first RTCP from the server",
        );
        first(
            &mut self.undecryptable,
            feedback.undecryptable,
            "got RTCP the transport key couldn't open",
        );
        first(
            &mut self.keyframe_requested,
            feedback.keyframe,
            "first keyframe request from a viewer",
        );
        first(
            &mut self.nacked,
            !feedback.lost.is_empty(),
            "first lost-packet report",
        );

        self.keyframe_requests += u64::from(feedback.keyframe);
        self.lost_reported += feedback.lost.len() as u64;
        for (kind, bytes) in &feedback.kinds {
            *self.kinds.entry(kind.clone()).or_default() += 1;
            if self.dumped.insert(kind.clone()) {
                let shown = &bytes[..bytes.len().min(64)];
                eprintln!(
                    "screen share: first {kind}, {} bytes: {shown:02X?}",
                    bytes.len()
                );
            }
        }
        for (ssrc, report) in &feedback.reports {
            self.reports.insert(*ssrc, *report);
        }
    }

    /// Logs the first frame that goes out: what it held before and after
    /// DAVE, which is where a frame no viewer can use would show.
    fn note_sent(&mut self, plain: &[u8], protected: &[u8], keyframe: bool) {
        self.sent += 1;
        self.keyframes_sent += u64::from(keyframe);
        if self.first_sent {
            return;
        }
        self.first_sent = true;
        let units: Vec<String> = nal_units(plain)
            .iter()
            .map(|unit| format!("{}({}b)", unit[0] & 0x1F, unit.len()))
            .collect();
        let tail = &protected[protected.len().saturating_sub(4)..];
        eprintln!(
            "screen share: first frame out: keyframe {keyframe}, NAL units {}, {} bytes plain, {} encrypted, ending {tail:02X?}",
            units.join(" "),
            plain.len(),
            protected.len(),
        );
    }

    fn summarize(&mut self, media: &Media, dave_ready: bool) {
        let now = media.counters.clone();
        let then = std::mem::replace(&mut self.previous, now.clone());
        let kinds: Vec<String> = std::mem::take(&mut self.kinds)
            .into_iter()
            .map(|(kind, count)| format!("{kind} x{count}"))
            .collect();
        eprintln!(
            "screen share: [{}s] frames: {} captured ({} key), {} sent ({} key), {} held for encryption, {} failed to encrypt | \
             out: {} packets, {} KB, {} send errors{} | in: {} packets, {} RTCP, {} undecryptable, {} other | \
             viewers asked for {} keyframes, {} resends | DAVE ready: {dave_ready}",
            SUMMARY_INTERVAL.as_secs(),
            self.captured,
            self.keyframes,
            self.sent,
            self.keyframes_sent,
            self.held,
            self.failed,
            now.packets - then.packets,
            (now.bytes - then.bytes) / 1024,
            now.send_errors - then.send_errors,
            now.last_error
                .as_deref()
                .map(|err| format!(" (last: {err})"))
                .unwrap_or_default(),
            now.received - then.received,
            now.rtcp - then.rtcp,
            now.undecryptable - then.undecryptable,
            now.other - then.other,
            self.keyframe_requests,
            self.lost_reported,
        );
        if !kinds.is_empty() {
            eprintln!("screen share:   RTCP seen: {}", kinds.join(", "));
        }
        eprintln!(
            "screen share:   we're at video seq {}",
            media.next_sequence().wrapping_sub(1)
        );
        for (ssrc, report) in std::mem::take(&mut self.reports) {
            eprintln!(
                "screen share:   server's report on SSRC {ssrc}: up to seq {} ({} wraps), {}/256 lost lately, {} lost in all, jitter {}",
                report.highest_sequence & 0xFFFF,
                report.highest_sequence >> 16,
                report.fraction_lost,
                report.cumulative_lost,
                report.jitter,
            );
        }
        self.captured = 0;
        self.keyframes = 0;
        self.sent = 0;
        self.keyframes_sent = 0;
        self.held = 0;
        self.failed = 0;
        self.keyframe_requests = 0;
        self.lost_reported = 0;
    }
}
