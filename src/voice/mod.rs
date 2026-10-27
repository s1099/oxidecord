//! Voice calls: songbird handles the handshake, transport, encryption and Opus;
//! this module feeds it the gateway's parameters and wires up [`audio`].
//!
//! Runs on the shared Tokio runtime. [`VoiceEngine`] is the only handle the app
//! holds (commands down, events up), so no songbird type reaches gpui's thread.
//! Screen sharing lives in [`stream`] because songbird can't send video.

mod audio;
pub mod stream;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use songbird::driver::{DecodeMode, Driver};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use songbird::id::{ChannelId, GuildId, UserId};
use songbird::tracks::TrackHandle;
use songbird::{Config, ConnectionInfo, error::ConnectionError};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::platform::runtime;

use audio::{AudioIo, Sender};

pub use audio::{input_devices, output_devices};

pub struct VoiceConnection {
    pub channel_id: Id<ChannelMarker>,
    /// `None` for a DM call; Discord expects the channel's own id in its place.
    pub guild_id: Option<Id<GuildMarker>>,
    pub user_id: Id<UserMarker>,
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
}

pub enum VoiceEvent {
    Connected,
    /// The call ended without the user asking.
    Disconnected,
    /// Sent only when the set changes.
    Speaking(HashSet<Id<UserMarker>>),
    Failed(String),
}

/// Drives one call at a time. Dropping it ends the task and any live call.
pub struct VoiceEngine {
    commands: UnboundedSender<Command>,
    speakers: CurrentAudio,
}

enum Command {
    Connect(Box<VoiceConnection>),
    Disconnect,
    Mute(bool),
    Deafen(bool),
    InputDevice(Option<String>),
    OutputDevice(Option<String>),
}

impl VoiceEngine {
    pub fn start(events: UnboundedSender<VoiceEvent>) -> Self {
        let (commands, rx) = unbounded();
        let speakers = CurrentAudio::default();
        runtime::handle().spawn(run(rx, events, speakers.clone()));
        Self { commands, speakers }
    }

    /// The call's current speakers; a no-op while there's no call.
    pub fn stream_audio(&self) -> StreamAudio {
        StreamAudio {
            speakers: self.speakers.clone(),
        }
    }

    pub fn connect(&self, connection: VoiceConnection) {
        self.send(Command::Connect(Box::new(connection)));
    }

    pub fn disconnect(&self) {
        self.send(Command::Disconnect);
    }

    /// The gateway is told separately; that is what shows the muted badge to others.
    pub fn set_muted(&self, muted: bool) {
        self.send(Command::Mute(muted));
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.send(Command::Deafen(deafened));
    }

    /// `None` follows the system default.
    pub fn set_input_device(&self, device: Option<String>) {
        self.send(Command::InputDevice(device));
    }

    /// `None` follows the system default.
    pub fn set_output_device(&self, device: Option<String>) {
        self.send(Command::OutputDevice(device));
    }

    fn send(&self, command: Command) {
        let _ = self.commands.unbounded_send(command);
    }
}

struct Call {
    driver: Driver,
    audio: SharedAudio,
    /// Held so it can be replaced when the device changes.
    mic: TrackHandle,
}

impl Drop for Call {
    fn drop(&mut self) {
        // Driver handlers also hold the devices and outlive it, so close them here.
        if let Ok(audio) = self.audio.lock() {
            audio.stop();
        }
    }
}

/// Switching a device reopens both, so handlers go through the inner `Arc` to
/// reach the current pair; a private clone would keep feeding a closed one.
type SharedAudio = Arc<Mutex<Arc<AudioIo>>>;

type CurrentAudio = Arc<Mutex<Option<SharedAudio>>>;

/// A watched stream's way into the call's speakers.
///
/// Plays through the call's devices and mixer so it follows the chosen output,
/// goes silent while deafened, and mixes like another voice.
#[derive(Clone)]
pub struct StreamAudio {
    speakers: CurrentAudio,
}

impl StreamAudio {
    /// `opus` must already be fully decrypted.
    pub fn receive(&self, ssrc: u32, sequence: u16, opus: &[u8]) {
        self.with_audio(|audio| audio.receive(Sender::Stream(ssrc), sequence, opus));
    }

    pub fn forget(&self, ssrc: u32) {
        self.with_audio(|audio| audio.forget(Sender::Stream(ssrc)));
    }

    fn with_audio(&self, act: impl FnOnce(&AudioIo)) {
        let Some(shared) = self
            .speakers
            .lock()
            .ok()
            .and_then(|current| current.clone())
        else {
            return;
        };
        if let Ok(audio) = shared.lock() {
            act(&audio);
        }
    }
}

/// Packets carry only an SSRC; the user mapping arrives in speaking updates.
#[derive(Default)]
struct Speakers {
    users: HashMap<u32, Id<UserMarker>>,
    audible: HashSet<Id<UserMarker>>,
}

async fn run(
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<VoiceEvent>,
    current: CurrentAudio,
) {
    use futures::StreamExt as _;

    let mut call: Option<Call> = None;
    let mut muted = false;
    let mut deafened = false;
    let mut devices = Devices::default();

    while let Some(command) = commands.next().await {
        match command {
            Command::Connect(connection) => {
                // Drop the old call first so its devices close before the new ones open.
                call = None;

                match connect(*connection, events.clone(), muted, deafened, &devices).await {
                    Ok(new) => {
                        call = Some(new);
                        let _ = events.unbounded_send(VoiceEvent::Connected);
                    }
                    Err(error) => {
                        let _ = events.unbounded_send(VoiceEvent::Failed(error.to_string()));
                    }
                }
            }
            Command::Disconnect => {
                if let Some(call) = &mut call {
                    call.driver.leave();
                }
                call = None;
            }
            Command::Mute(value) => {
                muted = value;
                if let Some(call) = &mut call {
                    call.driver.mute(value);
                }
            }
            Command::Deafen(value) => {
                deafened = value;
                if let Some(call) = &call
                    && let Ok(audio) = call.audio.lock()
                {
                    audio.set_deafened(value);
                }
            }
            Command::InputDevice(device) => {
                devices.input = device;
                if let Some(call) = &mut call {
                    reopen_devices(call, &devices, deafened);
                }
            }
            Command::OutputDevice(device) => {
                devices.output = device;
                if let Some(call) = &mut call {
                    reopen_devices(call, &devices, deafened);
                }
            }
        }

        if let Ok(mut current) = current.lock() {
            *current = call.as_ref().map(|call| call.audio.clone());
        }
    }
}

async fn connect(
    connection: VoiceConnection,
    events: UnboundedSender<VoiceEvent>,
    muted: bool,
    deafened: bool,
    devices: &Devices,
) -> Result<Call, ConnectionError> {
    // Decrypt only; packets are decoded in `audio`.
    let config = Config::default().decode_mode(DecodeMode::Decrypt);
    let mut driver = Driver::new(config);

    let audio = Arc::new(devices.open());
    audio.set_deafened(deafened);
    let mic_input = audio.mic_input();
    let audio: SharedAudio = Arc::new(Mutex::new(audio));
    let speakers = Arc::new(Mutex::new(Speakers::default()));

    driver.add_global_event(
        Event::Core(CoreEvent::SpeakingStateUpdate),
        SpeakingHandler {
            speakers: speakers.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::ClientDisconnect),
        DisconnectHandler {
            audio: audio.clone(),
            speakers: speakers.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::RtpPacket),
        PacketHandler {
            audio: audio.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::VoiceTick),
        TickHandler {
            speakers: speakers.clone(),
            events: events.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::DriverDisconnect),
        DriverDisconnectHandler {
            events: events.clone(),
        },
    );

    driver.connect(info(&connection)).await?;

    driver.mute(muted);
    let mic = driver.play_input(mic_input);

    Ok(Call { driver, audio, mic })
}

/// Ids as reported by [`input_devices`] and [`output_devices`]; `None` is the default.
#[derive(Default)]
struct Devices {
    input: Option<String>,
    output: Option<String>,
}

impl Devices {
    fn open(&self) -> AudioIo {
        AudioIo::start(self.input.as_deref(), self.output.as_deref())
    }
}

/// Reopens both devices and hands the call the new microphone.
///
/// The old track can't be repointed: it told the mixer the old sample rate. The
/// pair shares one thread, so both reopen, costing a few ms of playback.
fn reopen_devices(call: &mut Call, devices: &Devices, deafened: bool) {
    let Ok(mut current) = call.audio.lock() else {
        return;
    };

    let audio = Arc::new(devices.open());
    audio.set_deafened(deafened);
    let mic_input = audio.mic_input();

    let _ = call.mic.stop();
    // Close the old pair only after the new one is open to keep the gap short.
    current.stop();
    *current = audio;
    drop(current);

    call.mic = call.driver.play_input(mic_input);
}

/// A DM call has no guild; the voice `server_id` is then the channel's own id.
fn info(connection: &VoiceConnection) -> ConnectionInfo {
    // songbird uses a different twilight release, so convert via the raw value.
    ConnectionInfo {
        channel_id: ChannelId::from(connection.channel_id.into_nonzero()),
        endpoint: connection.endpoint.clone(),
        guild_id: GuildId::from(
            connection
                .guild_id
                .map_or(connection.channel_id.into_nonzero(), |id| id.into_nonzero()),
        ),
        session_id: connection.session_id.clone(),
        token: connection.token.clone(),
        user_id: UserId::from(connection.user_id.into_nonzero()),
    }
}

struct SpeakingHandler {
    speakers: Arc<Mutex<Speakers>>,
}

#[async_trait]
impl EventHandler for SpeakingHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::SpeakingStateUpdate(speaking) = ctx
            && let Some(user_id) = speaking.user_id
            && let Ok(mut speakers) = self.speakers.lock()
            && let Some(id) = Id::new_checked(user_id.0)
        {
            speakers.users.insert(speaking.ssrc, id);
        }
        None
    }
}

struct DisconnectHandler {
    audio: SharedAudio,
    speakers: Arc<Mutex<Speakers>>,
}

#[async_trait]
impl EventHandler for DisconnectHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let EventContext::ClientDisconnect(disconnect) = ctx else {
            return None;
        };
        let gone = Id::new_checked(disconnect.user_id.0)?;

        let streams: Vec<u32> = match self.speakers.lock() {
            Ok(mut speakers) => {
                speakers.audible.remove(&gone);
                let streams = speakers
                    .users
                    .iter()
                    .filter(|(_, id)| **id == gone)
                    .map(|(ssrc, _)| *ssrc)
                    .collect();
                speakers.users.retain(|_, id| *id != gone);
                streams
            }
            Err(_) => return None,
        };

        if let Ok(audio) = self.audio.lock() {
            for ssrc in streams {
                audio.forget(Sender::Call(ssrc));
            }
        }
        None
    }
}

/// songbird has already removed transport encryption and DAVE, and its offsets
/// skip the header extension, DAVE trailer and RTP padding, leaving the Opus frame.
struct PacketHandler {
    audio: SharedAudio,
}

#[async_trait]
impl EventHandler for PacketHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let EventContext::RtpPacket(packet) = ctx else {
            return None;
        };

        // Fixed header plus 4 bytes per CSRC; songbird's offsets count from there.
        let raw = &packet.packet[..];
        let header = 12 + 4 * usize::from(*raw.first()? & 0x0f);
        let sequence = u16::from_be_bytes([*raw.get(2)?, *raw.get(3)?]);
        let ssrc = u32::from_be_bytes(raw.get(8..12)?.try_into().ok()?);
        let payload = raw.get(header..)?;
        let end = payload.len().checked_sub(packet.payload_end_pad)?;
        let opus = payload.get(packet.payload_offset..end)?;

        if let Ok(audio) = self.audio.lock() {
            audio.receive(Sender::Call(ssrc), sequence, opus);
        }
        None
    }
}

struct TickHandler {
    speakers: Arc<Mutex<Speakers>>,
    events: UnboundedSender<VoiceEvent>,
}

#[async_trait]
impl EventHandler for TickHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let EventContext::VoiceTick(tick) = ctx else {
            return None;
        };

        let audible: HashSet<_> = match self.speakers.lock() {
            Ok(speakers) => tick
                .speaking
                .keys()
                .filter_map(|ssrc| speakers.users.get(ssrc).copied())
                .collect(),
            Err(_) => return None,
        };

        // 50 ticks a second; only send changes so tiles don't repaint needlessly.
        if let Ok(mut speakers) = self.speakers.lock()
            && speakers.audible != audible
        {
            speakers.audible.clone_from(&audible);
            let _ = self.events.unbounded_send(VoiceEvent::Speaking(audible));
        }

        None
    }
}

struct DriverDisconnectHandler {
    events: UnboundedSender<VoiceEvent>,
}

#[async_trait]
impl EventHandler for DriverDisconnectHandler {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let _ = self.events.unbounded_send(VoiceEvent::Disconnected);
        None
    }
}
