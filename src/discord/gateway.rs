//! The gateway websocket connection, which delivers live events.
//!
//! Dispatches are read as raw JSON rather than through twilight's `Event`
//! enum: the app needs a few user-client payloads twilight either models too
//! strictly (a DM call's `VOICE_SERVER_UPDATE` has no guild id) or doesn't
//! model at all. The shard still handles the session itself — heartbeats,
//! identify, resume — since that happens as messages are polled.

use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::value::RawValue;
use twilight_gateway::{Intents, Message as ShardMessage, MessageSender, Shard, ShardId};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, RoleMarker, UserMarker},
};

use crate::platform::runtime;

use super::model::{
    Message, RawRole, RawVoiceServer, RawVoiceState, Role, VoiceServerInfo, VoiceUserState,
    convert_message, convert_role, convert_voice_server, convert_voice_state,
};

/// A message received live over the gateway, tagged with the channel it
/// belongs to so the UI can decide whether it's for the open conversation.
pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    pub message: Message,
}

/// The live events the app acts on.
pub enum GatewayEvent {
    /// The session is up. Carries the signed-in user, which voice connections
    /// need to identify themselves.
    Ready {
        user_id: Id<UserMarker>,
    },
    Message(IncomingMessage),
    /// A message was edited. Carries the whole message as it now stands.
    MessageUpdate(IncomingMessage),
    /// Someone joined, left, or changed their state in a voice channel. Also
    /// synthesized for the states bundled into `GUILD_CREATE`, so the app
    /// learns who was already in a channel before it connected.
    VoiceState(VoiceUserState),
    /// The voice server assigned to a call the user is joining.
    VoiceServer(VoiceServerInfo),
    /// Every role in a guild, from `READY` or `GUILD_CREATE`. Replaces what
    /// was known before.
    GuildRoles {
        guild_id: Id<GuildMarker>,
        roles: Vec<Role>,
    },
    /// A role was created or changed.
    RoleUpdate {
        guild_id: Id<GuildMarker>,
        role: Role,
    },
    RoleDelete {
        guild_id: Id<GuildMarker>,
        role_id: Id<RoleMarker>,
    },
}

/// Sends commands up the gateway from outside the receive loop.
///
/// Cloneable and cheap: commands are queued on a channel the shard drains, so
/// they survive a reconnect rather than failing while one is in flight.
#[derive(Clone)]
pub struct GatewaySender {
    inner: MessageSender,
}

impl GatewaySender {
    /// Sends `VOICE_STATE_UPDATE` (opcode 4): joins `channel_id`, or leaves
    /// the current channel when it's `None`.
    ///
    /// `guild_id` is `None` for a DM call, which Discord accepts as a null
    /// field — twilight's own command type can't express that, so the payload
    /// is built here.
    pub fn update_voice_state(
        &self,
        guild_id: Option<Id<GuildMarker>>,
        channel_id: Option<Id<ChannelMarker>>,
        self_mute: bool,
        self_deaf: bool,
    ) {
        let payload = serde_json::json!({
            "op": 4,
            "d": {
                "guild_id": guild_id,
                "channel_id": channel_id,
                "self_mute": self_mute,
                "self_deaf": self_deaf,
                "self_video": false,
            }
        });
        let _ = self.inner.send(payload.to_string());
    }
}

/// The envelope every gateway payload arrives in. Only dispatches (opcode 0)
/// carry a name and data the app cares about. Both borrow from the frame, so
/// telling what a payload is costs no allocations — which matters, because
/// most of a user account's firehose (presences, typing) is dropped unread.
#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(default, borrow)]
    t: Option<&'a str>,
    #[serde(default, borrow)]
    d: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct ReadyPayload {
    user: ReadyUser,
    /// A user session's guilds arrive whole in `READY`; a bot's only as ids,
    /// with the rest following in `GUILD_CREATE`.
    #[serde(default)]
    guilds: Vec<GuildRolesPayload>,
}

#[derive(Deserialize)]
struct GuildRolesPayload {
    id: Id<GuildMarker>,
    #[serde(default)]
    roles: Vec<RawRole>,
}

/// `GUILD_ROLE_CREATE` and `GUILD_ROLE_UPDATE`.
#[derive(Deserialize)]
struct RoleUpdatePayload {
    guild_id: Id<GuildMarker>,
    role: RawRole,
}

#[derive(Deserialize)]
struct RoleDeletePayload {
    guild_id: Id<GuildMarker>,
    role_id: Id<RoleMarker>,
}

#[derive(Deserialize)]
struct ReadyUser {
    id: Id<UserMarker>,
}

/// `GUILD_CREATE`, of which only the voice states and roles are read here:
/// who is already sitting in each of the guild's voice channels, and what a
/// role mention should be called.
#[derive(Deserialize)]
struct GuildCreatePayload {
    id: Id<GuildMarker>,
    #[serde(default)]
    voice_states: Vec<RawVoiceState>,
    #[serde(default)]
    roles: Vec<RawRole>,
}

/// Opens a gateway websocket connection and invokes `on_event` for every
/// dispatch the app acts on. Returning `false` from it (the receiving end went
/// away) ends the shard loop and drops the socket.
///
/// The shard reconnects and resumes on its own, so transient errors are
/// skipped rather than treated as fatal.
pub fn connect_gateway(
    token: String,
    mut on_event: impl FnMut(GatewayEvent) -> bool + Send + 'static,
) -> GatewaySender {
    // The shard is built here rather than inside the task because the sender
    // has to come back to the caller — but building one starts the identify
    // queue's timer, so it has to happen inside the runtime all the same.
    let guard = runtime::handle().enter();
    // Discord ignores the intents field for user tokens; a real user client
    // receives every event its account can see. Request all intents so we
    // mirror that and never filter events at this layer (the shard still
    // requires a value in the IDENTIFY payload).
    let mut shard = Shard::new(ShardId::ONE, token, Intents::all());
    let sender = GatewaySender {
        inner: shard.sender(),
    };
    drop(guard);

    runtime::handle().spawn(async move {
        while let Some(item) = shard.next().await {
            // Reconnects and resumes are handled by the shard internally; a
            // receive error just means skip this one and keep listening.
            let Ok(ShardMessage::Text(json)) = item else {
                continue;
            };
            let Ok(Envelope {
                t: Some(name),
                d: Some(data),
            }) = serde_json::from_str::<Envelope>(&json)
            else {
                continue;
            };

            // A dispatch the app doesn't handle, or one whose shape doesn't
            // match, yields nothing rather than ending the loop.
            for event in dispatch(name, data) {
                if !on_event(event) {
                    return;
                }
            }
        }
    });

    sender
}

/// Turns one dispatch into the events the app acts on. `READY` and
/// `GUILD_CREATE` fan out into several.
fn dispatch(name: &str, data: &RawValue) -> Vec<GatewayEvent> {
    let data = data.get();
    match name {
        "READY" => serde_json::from_str::<ReadyPayload>(data)
            .map(|ready| {
                let roles = ready.guilds.into_iter().filter_map(guild_roles);
                std::iter::once(GatewayEvent::Ready {
                    user_id: ready.user.id,
                })
                .chain(roles)
                .collect()
            })
            .unwrap_or_default(),
        "MESSAGE_CREATE" => serde_json::from_str::<twilight_model::channel::Message>(data)
            .map(|message| {
                vec![GatewayEvent::Message(IncomingMessage {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                })]
            })
            .unwrap_or_default(),
        // twilight models this dispatch as a partial message, because Discord
        // hasn't always sent the whole thing. Parsing it as a full one takes
        // the updates that do and skips any that don't.
        "MESSAGE_UPDATE" => serde_json::from_str::<twilight_model::channel::Message>(data)
            .map(|message| {
                vec![GatewayEvent::MessageUpdate(IncomingMessage {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                })]
            })
            .unwrap_or_default(),
        "VOICE_STATE_UPDATE" => serde_json::from_str::<RawVoiceState>(data)
            .map(|state| vec![GatewayEvent::VoiceState(convert_voice_state(state, None))])
            .unwrap_or_default(),
        "VOICE_SERVER_UPDATE" => serde_json::from_str::<RawVoiceServer>(data)
            .map(|server| vec![GatewayEvent::VoiceServer(convert_voice_server(server))])
            .unwrap_or_default(),
        "GUILD_CREATE" => serde_json::from_str::<GuildCreatePayload>(data)
            .map(|guild| {
                let roles = guild_roles(GuildRolesPayload {
                    id: guild.id,
                    roles: guild.roles,
                });
                guild
                    .voice_states
                    .into_iter()
                    .map(|state| {
                        GatewayEvent::VoiceState(convert_voice_state(state, Some(guild.id)))
                    })
                    .chain(roles)
                    .collect()
            })
            .unwrap_or_default(),
        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            serde_json::from_str::<RoleUpdatePayload>(data)
                .map(|update| {
                    vec![GatewayEvent::RoleUpdate {
                        guild_id: update.guild_id,
                        role: convert_role(update.role),
                    }]
                })
                .unwrap_or_default()
        }
        "GUILD_ROLE_DELETE" => serde_json::from_str::<RoleDeletePayload>(data)
            .map(|delete| {
                vec![GatewayEvent::RoleDelete {
                    guild_id: delete.guild_id,
                    role_id: delete.role_id,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A guild's roles as an event, or nothing for a guild that arrived without
/// them (an unavailable one in `READY`).
fn guild_roles(guild: GuildRolesPayload) -> Option<GatewayEvent> {
    (!guild.roles.is_empty()).then(|| GatewayEvent::GuildRoles {
        guild_id: guild.id,
        roles: guild.roles.into_iter().map(convert_role).collect(),
    })
}
