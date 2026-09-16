//! Voice calls: the connection to Discord's voice servers and the audio that
//! travels over it.
//!
//! The websocket handshake, UDP transport, encryption, and Opus coding are
//! songbird's; this module supplies the parameters the gateway hands back,
//! wires the microphone and speakers to it ([`audio`]), and reports back what
//! the UI needs to draw.
//!
//! Everything here runs on the shared Tokio runtime. [`VoiceEngine`] is the
//! only handle the rest of the app holds: commands go down a channel, events
//! come back up another, so no songbird type crosses into gpui's thread.

mod audio;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use songbird::driver::{DecodeConfig, DecodeMode, Driver};
use songbird::events::{CoreEvent, Event, EventContext, EventHandler};
use songbird::id::{ChannelId, GuildId, UserId};
use songbird::tracks::TrackHandle;
use songbird::{Config, ConnectionInfo, error::ConnectionError};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};

use crate::platform::runtime;

use audio::AudioIo;

pub use audio::{InputDevice, input_devices};

/// Everything needed to open a voice connection, gathered from the pair of
/// gateway dispatches that follow a join.
pub struct VoiceConnection {
    pub channel_id: Id<ChannelMarker>,
    /// The guild the channel is in. A DM call has none, and Discord expects
    /// the channel's own id in its place.
    pub guild_id: Option<Id<GuildMarker>>,
    pub user_id: Id<UserMarker>,
    pub session_id: String,
    pub token: String,
    pub endpoint: String,
}

/// What the call reports back to the UI.
pub enum VoiceEvent {
    /// The voice connection is up and audio is flowing.
    Connected,
    /// The call ended on its own: the connection dropped, or the user was
    /// disconnected from the far end.
    Disconnected,
    /// Who is transmitting right now, sent whenever the set changes.
    Speaking(HashSet<Id<UserMarker>>),
    /// The connection could not be established.
    Failed(String),
}

/// Drives one call at a time.
///
/// Dropping the engine closes the command channel, which ends the task and
/// tears down whatever call was live.
pub struct VoiceEngine {
    commands: UnboundedSender<Command>,
}

enum Command {
    Connect(Box<VoiceConnection>),
    Disconnect,
    Mute(bool),
    Deafen(bool),
    /// Switch to another microphone, by the id [`input_devices`] reported.
    InputDevice(Option<String>),
}

impl VoiceEngine {
    /// Starts the engine. Events are delivered on `events` until the receiver
    /// is dropped.
    pub fn start(events: UnboundedSender<VoiceEvent>) -> Self {
        let (commands, rx) = unbounded();
        runtime::handle().spawn(run(rx, events));
        Self { commands }
    }

    pub fn connect(&self, connection: VoiceConnection) {
        self.send(Command::Connect(Box::new(connection)));
    }

    pub fn disconnect(&self) {
        self.send(Command::Disconnect);
    }

    /// Stops sending the microphone. The gateway is told separately, which is
    /// what puts the muted badge on the user's tile for everyone else.
    pub fn set_muted(&self, muted: bool) {
        self.send(Command::Mute(muted));
    }

    /// Stops playing what everyone else is saying.
    pub fn set_deafened(&self, deafened: bool) {
        self.send(Command::Deafen(deafened));
    }

    /// Picks the microphone to send, taking effect immediately if a call is
    /// already up. `None` follows the system default.
    pub fn set_input_device(&self, device: Option<String>) {
        self.send(Command::InputDevice(device));
    }

    fn send(&self, command: Command) {
        let _ = self.commands.unbounded_send(command);
    }
}

/// The call as the engine task holds it: songbird's driver and the audio
/// devices feeding it. Dropping it ends the call.
struct Call {
    driver: Driver,
    audio: SharedAudio,
    /// The microphone, playing into the call as a track like any other. Held
    /// so it can be stopped and replaced when the device changes.
    mic: TrackHandle,
}

impl Drop for Call {
    fn drop(&mut self) {
        // The driver's event handlers hold the devices too, and outlive it by
        // however long its shutdown takes, so the devices are closed here
        // rather than when the last handle goes.
        if let Ok(audio) = self.audio.lock() {
            audio.stop();
        }
    }
}

/// The call's audio devices, as everything that touches them holds them.
///
/// Switching microphone reopens both devices, and the driver's tick handler
/// has to reach whichever pair is current — its own clone would otherwise go
/// on filling a closed one, and playback would stop at the first switch.
type SharedAudio = Arc<Mutex<Arc<AudioIo>>>;

/// Who owns which RTP stream, and who was audible on the last tick.
///
/// Packets identify their sender by SSRC only; the mapping to a user arrives
/// separately, in the voice websocket's speaking updates.
#[derive(Default)]
struct Speakers {
    users: HashMap<u32, Id<UserMarker>>,
    audible: HashSet<Id<UserMarker>>,
}

async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<VoiceEvent>) {
    use futures::StreamExt as _;

    let mut call: Option<Call> = None;
    let mut muted = false;
    let mut deafened = false;
    let mut input_device: Option<String> = None;

    while let Some(command) = commands.next().await {
        match command {
            Command::Connect(connection) => {
                // Only one call at a time: leaving the old one first stops its
                // devices before the new one opens them.
                call = None;

                match connect(
                    *connection,
                    events.clone(),
                    muted,
                    deafened,
                    input_device.as_deref(),
                )
                .await
                {
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
                input_device = device;
                if let Some(call) = &mut call {
                    swap_microphone(call, input_device.as_deref(), deafened);
                }
            }
        }
    }
}

async fn connect(
    connection: VoiceConnection,
    events: UnboundedSender<VoiceEvent>,
    muted: bool,
    deafened: bool,
    input_device: Option<&str>,
) -> Result<Call, ConnectionError> {
    // Decoding is off by default, since a bot usually only sends. Here every
    // packet has to become audio.
    let config = Config::default().decode_mode(DecodeMode::Decode(DecodeConfig::default()));
    let mut driver = Driver::new(config);

    let audio = Arc::new(AudioIo::start(input_device));
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
            speakers: speakers.clone(),
        },
    );
    driver.add_global_event(
        Event::Core(CoreEvent::VoiceTick),
        TickHandler {
            audio: audio.clone(),
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
    // The microphone plays into the call like any other track, and never ends.
    let mic = driver.play_input(mic_input);

    Ok(Call { driver, audio, mic })
}

/// Reopens the devices on another microphone and hands the call the new one.
///
/// The old track can't simply be repointed: it declared the old device's
/// sample rate to the mixer, and the new device rarely runs at the same one.
/// Reopening takes the speakers with it, which costs a few milliseconds of
/// playback — only when the user switches, and simpler than rebuilding half of
/// the audio thread underneath a live track.
fn swap_microphone(call: &mut Call, input_device: Option<&str>, deafened: bool) {
    let Ok(mut current) = call.audio.lock() else {
        return;
    };

    let audio = Arc::new(AudioIo::start(input_device));
    audio.set_deafened(deafened);
    let mic_input = audio.mic_input();

    let _ = call.mic.stop();
    // Closing the old pair after the new one is open keeps the gap down to the
    // few milliseconds the devices take to start.
    current.stop();
    *current = audio;
    drop(current);

    call.mic = call.driver.play_input(mic_input);
}

/// Songbird's connection parameters.
///
/// `guild_id` is only ever sent on as the voice server's `server_id`, which for
/// a DM call is the channel's own id — so a guildless call substitutes it and
/// connects the same way.
fn info(connection: &VoiceConnection) -> ConnectionInfo {
    // songbird is built against a different twilight release, so its id
    // conversions don't apply to this crate's ids; the raw value is the only
    // thing they have in common.
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

/// Learns which user an RTP stream belongs to.
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

/// Forgets a user's stream when they leave the call.
struct DisconnectHandler {
    speakers: Arc<Mutex<Speakers>>,
}

#[async_trait]
impl EventHandler for DisconnectHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::ClientDisconnect(disconnect) = ctx
            && let Ok(mut speakers) = self.speakers.lock()
        {
            let gone = Id::new_checked(disconnect.user_id.0);
            speakers.users.retain(|_, id| Some(*id) != gone);
            if let Some(gone) = gone {
                speakers.audible.remove(&gone);
            }
        }
        None
    }
}

/// One 20ms tick of the call: every speaker's decoded audio, mixed and queued
/// for the speakers.
struct TickHandler {
    audio: SharedAudio,
    speakers: Arc<Mutex<Speakers>>,
    events: UnboundedSender<VoiceEvent>,
}

#[async_trait]
impl EventHandler for TickHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let EventContext::VoiceTick(tick) = ctx else {
            return None;
        };

        // Summing is what mixing several speakers into one stream means. The
        // sum is kept wider than the samples that went into it so the peaks it
        // produces survive as far as `play`, which eases them back inside full
        // scale — clipping them flat here is what a busy call is heard as.
        let mut mixed: Vec<i32> = Vec::new();
        let mut audible = HashSet::new();

        // One lock for the whole tick: this runs fifty times a second, and the
        // mapping can't change underneath it in the middle either way.
        let known = match self.speakers.lock() {
            Ok(speakers) => speakers.users.clone(),
            Err(_) => return None,
        };

        for (ssrc, data) in &tick.speaking {
            if let Some(user_id) = known.get(ssrc) {
                audible.insert(*user_id);
            }

            let Some(voice) = &data.decoded_voice else {
                continue;
            };
            if mixed.len() < voice.len() {
                mixed.resize(voice.len(), 0);
            }
            for (slot, sample) in mixed.iter_mut().zip(voice) {
                *slot += i32::from(*sample);
            }
        }

        if !mixed.is_empty()
            && let Ok(audio) = self.audio.lock()
        {
            audio.play(&mixed);
        }

        // Tiles only repaint when the set changes; at 50 ticks a second,
        // sending every one would repaint the call for no reason.
        if let Ok(mut speakers) = self.speakers.lock()
            && speakers.audible != audible
        {
            speakers.audible.clone_from(&audible);
            let _ = self.events.unbounded_send(VoiceEvent::Speaking(audible));
        }

        None
    }
}

/// Reports a call that ended without the user asking.
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
