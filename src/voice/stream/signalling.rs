//! The stream's voice websocket (gateway v8, the stream's own server and
//! token): handshake, heartbeats and DAVE. Spoken directly because songbird
//! can't announce video; the DAVE handshake is ported from songbird's, with
//! davey doing the MLS.
//!
//! DAVE is an MLS group per connection: the server proposes members, one
//! commits, and every member then derives the same keys. Until
//! [`Dave::protect`] has a ready session frames are held back, not sent.
//!
//! A viewer announces it sends nothing and learns the streamer's SSRCs from
//! the server's `VIDEO` messages.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use davey::{Codec, DaveSession, MediaType, ProposalsOperationType};
use futures::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_websockets::{MaybeTlsStream, Message, WebSocketStream};

use super::StreamConnection;
use super::media::{AES_MODE, H264_PAYLOAD, OPUS_PAYLOAD, RTX_PAYLOAD, Ssrcs, XCHACHA_MODE};

const VOICE_GATEWAY_VERSION: u8 = 8;

mod op {
    pub const IDENTIFY: u8 = 0;
    pub const SELECT_PROTOCOL: u8 = 1;
    pub const READY: u8 = 2;
    pub const HEARTBEAT: u8 = 3;
    pub const SESSION_DESCRIPTION: u8 = 4;
    pub const SPEAKING: u8 = 5;
    pub const HELLO: u8 = 8;
    pub const MEDIA_SINK_WANTS: u8 = 15;
    pub const CLIENTS_CONNECT: u8 = 11;
    pub const VIDEO: u8 = 12;
    pub const CLIENT_DISCONNECT: u8 = 13;
    pub const DAVE_PREPARE_TRANSITION: u8 = 21;
    pub const DAVE_EXECUTE_TRANSITION: u8 = 22;
    pub const DAVE_TRANSITION_READY: u8 = 23;
    pub const DAVE_PREPARE_EPOCH: u8 = 24;
    pub const MLS_EXTERNAL_SENDER: u8 = 25;
    pub const MLS_KEY_PACKAGE: u8 = 26;
    pub const MLS_PROPOSALS: u8 = 27;
    pub const MLS_COMMIT_WELCOME: u8 = 28;
    pub const MLS_ANNOUNCE_COMMIT_TRANSITION: u8 = 29;
    pub const MLS_WELCOME: u8 = 30;
    pub const MLS_INVALID_COMMIT_WELCOME: u8 = 31;
}

/// The server ended it on purpose: stream deleted, or user disconnected from the call.
const CLOSE_DISCONNECTED: u16 = 4014;

pub(super) struct Ready {
    pub ip: IpAddr,
    pub port: u16,
    pub ssrcs: Ssrcs,
    pub mode: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct RemoteVideo {
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
}

pub(super) enum Signal {
    Nothing,
    /// `None` when the server ended the stream on purpose.
    Closed(Option<String>),
}

pub(super) struct Signalling {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    /// Acked on every heartbeat; v8 replays anything missed past it on resume.
    seq_ack: i64,
    pub heartbeat: Duration,
    pub dave: Dave,
    /// DAVE messages that arrived before the session description (which says
    /// whether there is a DAVE session). Replayed once it lands.
    early: Option<Vec<Vec<u8>>>,
    /// Not a [`Signal`] because the server can announce it during the handshake,
    /// before anyone reads signals.
    pub remote: Option<RemoteVideo>,
}

#[derive(Deserialize)]
struct Envelope {
    op: u8,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    seq: Option<i64>,
}

#[derive(Deserialize)]
struct HelloPayload {
    heartbeat_interval: f64,
}

#[derive(Deserialize)]
struct ReadyPayload {
    ssrc: u32,
    ip: IpAddr,
    port: u16,
    modes: Vec<String>,
    #[serde(default)]
    streams: Vec<ReadyStream>,
}

#[derive(Deserialize)]
struct ReadyStream {
    ssrc: u32,
    rtx_ssrc: u32,
}

#[derive(Deserialize)]
struct SessionDescription {
    mode: String,
    secret_key: Vec<u8>,
    #[serde(default)]
    dave_protocol_version: u16,
}

#[derive(Deserialize)]
struct VideoPayload {
    #[serde(default)]
    video_ssrc: u32,
    #[serde(default)]
    streams: Vec<VideoStream>,
}

#[derive(Deserialize)]
struct VideoStream {
    #[serde(default)]
    ssrc: u32,
    #[serde(default)]
    rtx_ssrc: Option<u32>,
}

#[derive(Deserialize)]
struct ClientsConnect {
    user_ids: Vec<String>,
}

#[derive(Deserialize)]
struct ClientDisconnect {
    user_id: String,
}

#[derive(Deserialize)]
struct Transition {
    transition_id: u16,
    #[serde(default)]
    protocol_version: u16,
}

#[derive(Deserialize)]
struct Epoch {
    epoch: u64,
    protocol_version: u16,
}

impl Signalling {
    /// Returns once the server has said hello and where to send media.
    pub(super) async fn connect(connection: &StreamConnection) -> Result<(Self, Ready), String> {
        let url = format!("wss://{}/?v={VOICE_GATEWAY_VERSION}", connection.endpoint);
        let (ws, _) = tokio_websockets::ClientBuilder::new()
            .uri(&url)
            .map_err(|err| format!("The stream server's address is invalid: {err}"))?
            .connect()
            .await
            .map_err(|err| format!("Couldn't connect to the stream server: {err}"))?;

        let user_id = connection.user_id.get();
        let mut this = Self {
            ws,
            seq_ack: -1,
            heartbeat: Duration::from_secs(10),
            dave: Dave::new(user_id, connection.dave_channel_id),
            early: Some(Vec::new()),
            remote: None,
        };

        this.send_json(
            op::IDENTIFY,
            json!({
                "server_id": connection.server_id,
                "user_id": user_id.to_string(),
                "session_id": connection.session_id,
                "token": connection.token,
                "video": true,
                // One full-quality stream; simulcast would mean encoding each size.
                "streams": [{ "type": "screen", "rid": "100", "quality": 100 }],
                "max_dave_protocol_version": davey::DAVE_PROTOCOL_VERSION,
            }),
        )
        .await?;

        let mut hello = None;
        let mut ready = None;
        while hello.is_none() || ready.is_none() {
            let Some(envelope) = this.next_json().await? else {
                return Err("The stream server closed the connection".into());
            };
            match envelope.op {
                op::HELLO => {
                    let payload: HelloPayload = parse(envelope.d)?;
                    hello = Some(Duration::from_secs_f64(payload.heartbeat_interval / 1000.));
                }
                op::READY => ready = Some(parse::<ReadyPayload>(envelope.d)?),
                _ => {
                    this.handle_json(envelope).await?;
                }
            }
        }
        let (Some(heartbeat), Some(ready)) = (hello, ready) else {
            unreachable!("the loop only ends once both have arrived");
        };
        this.heartbeat = heartbeat;

        // Only one stream was offered.
        let stream = ready
            .streams
            .first()
            .ok_or("The stream server assigned no video stream")?;
        // AES-GCM where offered, as Discord asks; xchacha is the one every server supports.
        let mode = if ready.modes.iter().any(|mode| mode == AES_MODE) {
            AES_MODE
        } else if ready.modes.iter().any(|mode| mode == XCHACHA_MODE) {
            XCHACHA_MODE
        } else {
            return Err("The stream server offered no cipher this app supports".into());
        };

        let ready = Ready {
            ip: ready.ip,
            port: ready.port,
            ssrcs: Ssrcs {
                audio: ready.ssrc,
                video: stream.ssrc,
                rtx: stream.rtx_ssrc,
            },
            mode,
        };
        Ok((this, ready))
    }

    /// Returns the transport cipher and its key.
    pub(super) async fn select_protocol(
        &mut self,
        external: SocketAddr,
        mode: &str,
    ) -> Result<(String, Vec<u8>), String> {
        self.send_json(
            op::SELECT_PROTOCOL,
            json!({
                "protocol": "udp",
                "codecs": [
                    { "name": "opus", "type": "audio", "priority": 1000, "payload_type": OPUS_PAYLOAD },
                    {
                        "name": "H264", "type": "video", "priority": 1000,
                        "payload_type": H264_PAYLOAD, "rtx_payload_type": RTX_PAYLOAD,
                        "encode": true, "decode": true,
                    },
                ],
                "data": {
                    "address": external.ip().to_string(),
                    "port": external.port(),
                    "mode": mode,
                },
            }),
        )
        .await?;

        loop {
            let Some(envelope) = self.next_json().await? else {
                return Err("The stream server closed the connection".into());
            };
            if envelope.op != op::SESSION_DESCRIPTION {
                self.handle_json(envelope).await?;
                continue;
            }

            let description: SessionDescription = parse(envelope.d)?;
            self.dave.protocol_version = description.dave_protocol_version;
            if let Some(key_package) = self.dave.reinit()? {
                self.send_binary(op::MLS_KEY_PACKAGE, &key_package).await?;
            }
            for message in self.early.take().unwrap_or_default() {
                self.handle_binary(&message).await?;
            }
            return Ok((description.mode, description.secret_key));
        }
    }

    pub(super) async fn announce_video(
        &mut self,
        ssrcs: Ssrcs,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
    ) -> Result<(), String> {
        self.send_json(
            op::VIDEO,
            json!({
                "audio_ssrc": ssrcs.audio,
                "video_ssrc": ssrcs.video,
                "rtx_ssrc": ssrcs.rtx,
                "streams": [{
                    "type": "video",
                    "rid": "100",
                    "ssrc": ssrcs.video,
                    "rtx_ssrc": ssrcs.rtx,
                    "active": true,
                    "quality": 100,
                    "max_bitrate": bitrate,
                    "max_framerate": fps,
                    "max_resolution": { "type": "fixed", "width": size.0, "height": size.1 },
                }],
            }),
        )
        .await?;

        // For a stream, "speaking" means soundshare, and the client shows the stream live on it.
        self.send_json(
            op::SPEAKING,
            json!({ "speaking": 2, "delay": 0, "ssrc": ssrcs.audio }),
        )
        .await
    }

    /// The server forwards no video until it has heard a `VIDEO` from each end.
    pub(super) async fn announce_watching(&mut self, ssrcs: Ssrcs) -> Result<(), String> {
        self.send_json(
            op::VIDEO,
            json!({
                "audio_ssrc": ssrcs.audio,
                "video_ssrc": 0,
                "rtx_ssrc": 0,
                "streams": [{
                    "type": "video",
                    "rid": "100",
                    "ssrc": ssrcs.video,
                    "rtx_ssrc": ssrcs.rtx,
                    "active": false,
                    "quality": 100,
                }],
            }),
        )
        .await?;
        self.send_json(op::MEDIA_SINK_WANTS, json!({ "any": 100 }))
            .await
    }

    pub(super) async fn send_heartbeat(&mut self) -> Result<(), String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.send_json(op::HEARTBEAT, json!({ "t": now, "seq_ack": self.seq_ack }))
            .await
    }

    pub(super) async fn close(&mut self) {
        let _ = self.ws.close().await;
    }

    pub(super) async fn next(&mut self) -> Result<Signal, String> {
        let Some(message) = self.ws.next().await else {
            return Ok(Signal::Closed(Some("The stream server went away".into())));
        };
        let message = message.map_err(|err| format!("Lost the stream server: {err}"))?;

        if let Some((code, reason)) = message.as_close() {
            let code = u16::from(code);
            return Ok(Signal::Closed(if code == CLOSE_DISCONNECTED {
                None
            } else {
                Some(format!(
                    "The stream server closed the connection ({code} {reason})"
                ))
            }));
        }
        if message.is_binary() {
            self.handle_binary(message.as_payload()).await?;
        } else if let Some(text) = message.as_text() {
            match serde_json::from_str::<Envelope>(text) {
                Ok(envelope) => {
                    self.note_seq(envelope.seq);
                    self.handle_json(envelope).await?;
                }
                Err(err) => eprintln!("unreadable stream gateway message: {err}"),
            }
        }
        Ok(Signal::Nothing)
    }

    /// Handles binary messages on the way. `None` when the connection closed.
    async fn next_json(&mut self) -> Result<Option<Envelope>, String> {
        loop {
            let Some(message) = self.ws.next().await else {
                return Ok(None);
            };
            let message = message.map_err(|err| format!("Lost the stream server: {err}"))?;
            if let Some((code, reason)) = message.as_close() {
                return Err(format!(
                    "The stream server refused the connection ({} {reason})",
                    u16::from(code)
                ));
            }
            if message.is_binary() {
                self.handle_binary(message.as_payload()).await?;
                continue;
            }
            let Some(text) = message.as_text() else {
                continue;
            };
            if let Ok(envelope) = serde_json::from_str::<Envelope>(text) {
                self.note_seq(envelope.seq);
                return Ok(Some(envelope));
            }
        }
    }

    fn note_seq(&mut self, seq: Option<i64>) {
        if let Some(seq) = seq {
            self.seq_ack = seq;
        }
    }

    async fn handle_json(&mut self, envelope: Envelope) -> Result<(), String> {
        match envelope.op {
            op::VIDEO => {
                if let Ok(payload) = parse::<VideoPayload>(envelope.d) {
                    let stream = payload
                        .streams
                        .iter()
                        .find(|stream| stream.ssrc == payload.video_ssrc)
                        .or_else(|| payload.streams.iter().find(|stream| stream.ssrc != 0));
                    // Zero SSRC with no streams is the sender clearing its video; keep
                    // what's known, since a resumed stream reuses the same SSRCs.
                    let ssrc = stream.map_or(payload.video_ssrc, |stream| stream.ssrc);
                    if ssrc != 0 {
                        self.remote = Some(RemoteVideo {
                            ssrc,
                            rtx_ssrc: stream
                                .and_then(|stream| stream.rtx_ssrc)
                                .filter(|&rtx| rtx != 0),
                        });
                    }
                }
            }
            op::CLIENTS_CONNECT => {
                if let Ok(payload) = parse::<ClientsConnect>(envelope.d) {
                    self.dave.members.extend(
                        payload
                            .user_ids
                            .iter()
                            .filter_map(|id| id.parse::<u64>().ok()),
                    );
                }
            }
            op::CLIENT_DISCONNECT => {
                if let Ok(payload) = parse::<ClientDisconnect>(envelope.d)
                    && let Ok(id) = payload.user_id.parse::<u64>()
                {
                    self.dave.members.remove(&id);
                }
            }
            op::DAVE_PREPARE_TRANSITION => {
                let transition: Transition = parse(envelope.d)?;
                self.dave
                    .pending
                    .insert(transition.transition_id, transition.protocol_version);
                if transition.transition_id == 0 {
                    self.dave.execute(0);
                } else {
                    self.send_transition_ready(transition.transition_id).await?;
                }
            }
            op::DAVE_EXECUTE_TRANSITION => {
                let transition: Transition = parse(envelope.d)?;
                self.dave.execute(transition.transition_id);
            }
            op::DAVE_PREPARE_EPOCH => {
                let epoch: Epoch = parse(envelope.d)?;
                // Epoch 1 is a fresh group.
                if epoch.epoch == 1 {
                    self.dave.protocol_version = epoch.protocol_version;
                    if let Some(key_package) = self.dave.reinit()? {
                        self.send_binary(op::MLS_KEY_PACKAGE, &key_package).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Binary frames: sequence number, opcode, payload.
    async fn handle_binary(&mut self, data: &[u8]) -> Result<(), String> {
        let [seq_high, seq_low, opcode, payload @ ..] = data else {
            return Ok(());
        };
        self.seq_ack = i64::from(u16::from_be_bytes([*seq_high, *seq_low]));
        if let Some(early) = &mut self.early {
            early.push(data.to_vec());
            return Ok(());
        }

        match *opcode {
            op::MLS_EXTERNAL_SENDER => {
                if let Some(session) = &mut self.dave.session
                    && let Err(err) = session.set_external_sender(payload)
                {
                    eprintln!("DAVE: couldn't set the external sender: {err}");
                }
            }
            op::MLS_PROPOSALS => {
                let [kind, proposals @ ..] = payload else {
                    return Ok(());
                };
                let kind = if *kind == 0 {
                    ProposalsOperationType::APPEND
                } else {
                    ProposalsOperationType::REVOKE
                };
                let members: Vec<u64> = self.dave.members.iter().copied().collect();
                let Some(session) = &mut self.dave.session else {
                    return Ok(());
                };
                match session.process_proposals(kind, proposals, Some(&members)) {
                    Ok(Some(commit)) => {
                        let mut message = commit.commit;
                        if let Some(welcome) = commit.welcome {
                            message.extend_from_slice(&welcome);
                        }
                        self.send_binary(op::MLS_COMMIT_WELCOME, &message).await?;
                    }
                    Ok(None) => {}
                    Err(err) => eprintln!(
                        "DAVE: couldn't process proposals ({} connected): {err}",
                        members.len()
                    ),
                }
            }
            op::MLS_ANNOUNCE_COMMIT_TRANSITION | op::MLS_WELCOME => {
                let [high, low, message @ ..] = payload else {
                    return Ok(());
                };
                let transition_id = u16::from_be_bytes([*high, *low]);
                let Some(session) = &mut self.dave.session else {
                    return Ok(());
                };
                let result = if *opcode == op::MLS_WELCOME {
                    session
                        .process_welcome(message)
                        .map_err(|err| err.to_string())
                } else {
                    session
                        .process_commit(message)
                        .map_err(|err| err.to_string())
                };

                match result {
                    Ok(()) => {
                        if transition_id != 0 {
                            self.dave
                                .pending
                                .insert(transition_id, self.dave.protocol_version);
                            self.send_transition_ready(transition_id).await?;
                        }
                    }
                    Err(err) => {
                        // The group moved on without us; rejoin with a fresh key package.
                        eprintln!("DAVE: couldn't join the group: {err}");
                        self.send_json(
                            op::MLS_INVALID_COMMIT_WELCOME,
                            json!({ "transition_id": transition_id }),
                        )
                        .await?;
                        if let Some(key_package) = self.dave.reinit()? {
                            self.send_binary(op::MLS_KEY_PACKAGE, &key_package).await?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn send_transition_ready(&mut self, transition_id: u16) -> Result<(), String> {
        self.send_json(
            op::DAVE_TRANSITION_READY,
            json!({ "transition_id": transition_id }),
        )
        .await
    }

    async fn send_json(&mut self, op: u8, d: Value) -> Result<(), String> {
        let text = json!({ "op": op, "d": d }).to_string();
        self.ws
            .send(Message::text(text))
            .await
            .map_err(|err| format!("Lost the stream server: {err}"))
    }

    async fn send_binary(&mut self, op: u8, payload: &[u8]) -> Result<(), String> {
        let mut data = Vec::with_capacity(payload.len() + 1);
        data.push(op);
        data.extend_from_slice(payload);
        self.ws
            .send(Message::binary(data))
            .await
            .map_err(|err| format!("Lost the stream server: {err}"))
    }
}

pub(super) struct Dave {
    user_id: u64,
    /// The stream server's id less one, as the voice server derives it; a group
    /// on any other id is one no viewer is in.
    channel_id: u64,
    pub(super) session: Option<DaveSession>,
    /// Zero when the connection isn't end-to-end encrypted at all.
    protocol_version: u16,
    /// Announced but unexecuted transitions, by id, with their target protocol version.
    pending: HashMap<u16, u16>,
    /// Including the user. Proposals adding anyone else are refused.
    members: HashSet<u64>,
}

impl Dave {
    fn new(user_id: u64, channel_id: u64) -> Self {
        Self {
            user_id,
            channel_id,
            session: None,
            protocol_version: 0,
            pending: HashMap::new(),
            members: HashSet::from([user_id]),
        }
    }

    /// Returns the key package to announce, if there's a session to join with.
    fn reinit(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(version) = NonZeroU16::new(self.protocol_version) else {
            if let Some(session) = &mut self.session {
                let _ = session.reset();
            }
            return Ok(None);
        };

        let session = match &mut self.session {
            Some(session) => {
                session
                    .reinit(version, self.user_id, self.channel_id, None)
                    .map_err(|err| format!("Couldn't start end-to-end encryption: {err}"))?;
                session
            }
            None => self.session.insert(
                DaveSession::new(version, self.user_id, self.channel_id, None)
                    .map_err(|err| format!("Couldn't start end-to-end encryption: {err}"))?,
            ),
        };
        session
            .create_key_package()
            .map(Some)
            .map_err(|err| format!("Couldn't start end-to-end encryption: {err}"))
    }

    fn execute(&mut self, transition_id: u16) {
        match self.pending.remove(&transition_id) {
            Some(version) => self.protocol_version = version,
            None => eprintln!("DAVE: asked to execute unknown transition {transition_id}"),
        }
    }

    pub(super) fn is_ready(&self) -> bool {
        self.protocol_version == 0 || self.session.as_ref().is_some_and(DaveSession::is_ready)
    }

    /// `None` when it can't be decrypted, most often because the group hasn't formed.
    pub(super) fn unprotect(
        &mut self,
        sender: u64,
        media: MediaType,
        frame: Vec<u8>,
    ) -> Option<Vec<u8>> {
        if self.protocol_version == 0 {
            return Some(frame);
        }
        let session = self.session.as_mut()?;
        // Every frame fails until the group forms; don't log each.
        if !session.is_ready() {
            return None;
        }
        match session.decrypt(sender, media, &frame) {
            Ok(frame) => Some(frame),
            Err(err) => {
                eprintln!("DAVE: couldn't decrypt a frame: {err}");
                None
            }
        }
    }

    /// Passes through when unencrypted. `None` while the group forms: anything
    /// sent then is undecryptable by viewers.
    pub(super) fn protect<'a>(&mut self, frame: &'a [u8]) -> Option<Cow<'a, [u8]>> {
        if self.protocol_version == 0 {
            return Some(Cow::Borrowed(frame));
        }
        let session = self.session.as_mut()?;
        if !session.is_ready() {
            return None;
        }
        match session.encrypt(MediaType::VIDEO, Codec::H264, frame) {
            Ok(frame) => Some(frame),
            Err(err) => {
                eprintln!("DAVE: couldn't encrypt a frame: {err}");
                None
            }
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|err| format!("Unexpected message from the stream server: {err}"))
}
