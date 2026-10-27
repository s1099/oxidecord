//! Go Live: streaming the screen into a call.
//!
//! A stream is a second voice connection with its own server and token from the
//! main gateway. The screen goes out as H.264 over RTP, DAVE-encrypted like the
//! call's audio. Viewers connect to the same server; the streamer only sees
//! the RTCP feedback it relays.
//!
//! Watching is the same connection reversed, in [`watch`].
//!
//! Websocket, UDP and DAVE session live in one task, so none needs a lock.
//! [`GoLive`] is the only handle; dropping it ends the stream.

mod media;
mod receive;
mod signalling;
mod sps;
mod watch;

pub use watch::WatchStream;

use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use twilight_model::id::{Id, marker::UserMarker};

use crate::platform::capture::{
    CaptureEvent, CaptureSettings, CaptureSource, EncodedFrame, ScreenCapture,
};
use crate::platform::runtime;

use media::Media;
use signalling::{Signal, Signalling};

/// Sender reports let viewers sync the stream's clock with theirs.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// Gathered from the main gateway's `STREAM_CREATE` and `STREAM_SERVER_UPDATE`.
pub struct StreamConnection {
    pub stream_key: String,
    pub server_id: String,
    /// Derived from [`Self::server_id`].
    pub dave_channel_id: u64,
    pub user_id: Id<UserMarker>,
    /// The call's voice session; a stream belongs to the session that is in the call.
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

pub enum StreamEvent {
    Live {
        stream_key: String,
    },
    /// `error` is `None` for a normal end (window closed, server ended it).
    Ended {
        stream_key: String,
        error: Option<String>,
    },
}

/// Dropping it stops the capture and closes the connection.
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
    let (mut signalling, ready) = Signalling::connect(&connection).await?;
    let (mut media, external) = Media::connect(ready.ip, ready.port, ready.ssrcs).await?;
    let (mode, key) = signalling.select_protocol(external, ready.mode).await?;
    media.set_key(&mode, &key)?;

    let (capture_events, mut captured) = unbounded();
    let capture = ScreenCapture::start(source, settings, capture_events);

    let mut heartbeat = tokio::time::interval(signalling.heartbeat);
    let mut reports = tokio::time::interval(REPORT_INTERVAL);
    let mut buffer = vec![0u8; 1500];
    // Nothing sends until DAVE's group forms; the first frame after must be a
    // keyframe so viewers have something to start on.
    let mut was_ready = false;

    let outcome = loop {
        tokio::select! {
            _ = &mut stopped => break Outcome::Stopped,
            _ = heartbeat.tick() => signalling.send_heartbeat().await?,
            _ = reports.tick() => media.send_report().await,
            signal = signalling.next() => match signal? {
                Signal::Nothing => {}
                Signal::Closed(None) => break Outcome::Ended,
                Signal::Closed(Some(err)) => return Err(err),
            },
            event = captured.next() => match event {
                Some(CaptureEvent::Started { width, height }) => {
                    eprintln!("screen share: live at {width}x{height}, {} fps", settings.fps);
                    signalling
                        .announce_video(ready.ssrcs, (width, height), settings.fps, settings.bitrate)
                        .await?;
                    let _ = events.unbounded_send(StreamEvent::Live {
                        stream_key: connection.stream_key.clone(),
                    });
                }
                Some(CaptureEvent::Frame(frame)) => {
                    let ready_now = signalling.dave.is_ready();
                    if ready_now && !was_ready {
                        eprintln!("screen share: encryption ready, sending video");
                        if !frame.keyframe {
                            capture.request_keyframe();
                        }
                    }
                    was_ready = ready_now;
                    send(&mut signalling, &mut media, frame).await;
                }
                Some(CaptureEvent::SourceClosed) | None => break Outcome::Ended,
                Some(CaptureEvent::Failed(err)) => return Err(err),
            },
            received = media.recv(&mut buffer) => {
                let Ok(length) = received else { continue };
                let feedback = media.feedback(&buffer[..length]);
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

async fn send(signalling: &mut Signalling, media: &mut Media, frame: EncodedFrame) {
    let frame_data = sps::rewrite_frame(&frame.data);
    let Some(protected) = signalling.dave.protect(&frame_data) else {
        return;
    };
    media.send_frame(&protected, frame.timestamp).await;
}
