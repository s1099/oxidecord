//! The gateway websocket connection, which delivers live events.
//!
//! Dispatches are read as raw JSON rather than through twilight's `Event`
//! enum: the app needs a few user-client payloads twilight either models too
//! strictly (a DM call's `VOICE_SERVER_UPDATE` has no guild id) or doesn't
//! model at all. The shard still handles the session itself — heartbeats,
//! identify, resume — since that happens as messages are polled.

use futures::StreamExt as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use twilight_gateway::{Intents, Message as ShardMessage, MessageSender, Shard, ShardId};
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
};

use crate::platform::runtime;

use super::model::{
    GuildEmoji, Message, RawEmoji, RawMember, RawRole, RawVoiceServer, RawVoiceState,
    ReactionEmoji, Role, VoiceServerInfo, VoiceUserState, convert_guild_emoji,
    convert_guild_voice_states, convert_message, convert_reaction_emoji, convert_role,
    convert_voice_server, convert_voice_state,
};

/// A message received live over the gateway, tagged with the channel it
/// belongs to so the UI can decide whether it's for the open conversation.
pub struct IncomingMessage {
    pub channel_id: Id<ChannelMarker>,
    pub message: Message,
    /// The nonce the sender attached, when this is a new message. Matches the
    /// id of the optimistic copy of a message sent from here.
    pub nonce: Option<u64>,
}

/// The live events the app acts on.
pub enum GatewayEvent {
    /// A new session is up. Carries the signed-in user, which voice
    /// connections need to identify themselves, and everyone in voice across
    /// its guilds — the whole of it, replacing whatever an earlier session
    /// knew, since anyone could have left while the app was disconnected.
    Ready {
        user_id: Id<UserMarker>,
        voice_states: Vec<VoiceUserState>,
    },
    Message(IncomingMessage),
    /// A message was edited. Carries the whole message as it now stands.
    MessageUpdate(IncomingMessage),
    /// One message was deleted, or several at once by a moderator.
    MessageDelete {
        channel_id: Id<ChannelMarker>,
        message_ids: Vec<Id<MessageMarker>>,
    },
    /// A message's reactions changed.
    Reaction {
        channel_id: Id<ChannelMarker>,
        message_id: Id<MessageMarker>,
        change: ReactionChange,
    },
    /// Someone joined, left, or changed their state in a voice channel.
    VoiceState(VoiceUserState),
    /// Everyone in voice in one guild, replacing what was known of it: from
    /// `GUILD_CREATE`, or empty from `GUILD_DELETE`.
    GuildVoiceStates {
        guild_id: Id<GuildMarker>,
        states: Vec<VoiceUserState>,
    },
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
    /// Every custom emoji in a guild, from `READY`, `GUILD_CREATE`, or
    /// `GUILD_EMOJIS_UPDATE`. Replaces what was known before.
    GuildEmojis {
        guild_id: Id<GuildMarker>,
        emojis: Vec<GuildEmoji>,
    },
    /// A guild member's roles, from `READY`, `GUILD_CREATE`, or
    /// `GUILD_MEMBER_UPDATE`. Not only the signed-in user's — a user session
    /// is sent other members too — so the receiver picks out its own.
    MemberRoles {
        guild_id: Id<GuildMarker>,
        user_id: Id<UserMarker>,
        roles: Vec<Id<RoleMarker>>,
    },
}

/// How a message's reactions changed.
pub enum ReactionChange {
    /// Someone reacted. Includes the signed-in user, whose own reactions also
    /// come back as dispatches.
    Add {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    Remove {
        user_id: Id<UserMarker>,
        emoji: ReactionEmoji,
    },
    /// Every reaction with one emoji was cleared by a moderator.
    RemoveEmoji(ReactionEmoji),
    /// Every reaction on the message was cleared by a moderator.
    RemoveAll,
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
    guilds: Vec<GatewayGuild>,
    /// The signed-in user's member in each guild, in the same order as
    /// `guilds`, when the session asked for deduplicated payloads. Otherwise
    /// they arrive in each guild's own `members`.
    #[serde(default)]
    merged_members: Vec<Vec<RawMember>>,
}

/// A guild as `READY` and `GUILD_CREATE` send it, of which only what's needed
/// is read: who is already sitting in its voice channels, what a role mention
/// should be called, and which roles the user holds. An unavailable guild
/// arrives with none of these.
#[derive(Deserialize)]
struct GatewayGuild {
    id: Id<GuildMarker>,
    /// Lenient because it sits inside `READY`: one state Discord shapes
    /// unexpectedly shouldn't cost the whole session its roles.
    #[serde(default, deserialize_with = "skip_invalid")]
    voice_states: Vec<RawVoiceState>,
    #[serde(default)]
    roles: Vec<RawRole>,
    #[serde(default)]
    members: Vec<RawMember>,
    /// `None` for an unavailable guild, which says nothing about its emoji,
    /// as opposed to a guild that has none.
    #[serde(default, deserialize_with = "skip_invalid_opt")]
    emojis: Option<Vec<RawEmoji>>,
}

/// `GUILD_EMOJIS_UPDATE`: the guild's whole emoji list as it now stands.
#[derive(Deserialize)]
struct EmojisUpdatePayload {
    guild_id: Id<GuildMarker>,
    #[serde(deserialize_with = "skip_invalid")]
    emojis: Vec<RawEmoji>,
}

/// `GUILD_DELETE`: the user left the guild, or it went unavailable.
#[derive(Deserialize)]
struct GuildDeletePayload {
    id: Id<GuildMarker>,
}

/// `GUILD_MEMBER_UPDATE`.
#[derive(Deserialize)]
struct MemberUpdatePayload {
    guild_id: Id<GuildMarker>,
    #[serde(flatten)]
    member: RawMember,
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

/// The `nonce` on a `MESSAGE_CREATE`. Discord echoes it as whatever the sender
/// used, a string or a number, and other clients' nonces needn't be numeric.
#[derive(Deserialize)]
struct NoncePayload {
    #[serde(default, deserialize_with = "deserialize_nonce")]
    nonce: Option<u64>,
}

fn deserialize_nonce<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::String(nonce)) => nonce.parse().ok(),
            Some(serde_json::Value::Number(nonce)) => nonce.as_u64(),
            _ => None,
        },
    )
}

/// `MESSAGE_DELETE`.
#[derive(Deserialize)]
struct MessageDeletePayload {
    id: Id<MessageMarker>,
    channel_id: Id<ChannelMarker>,
}

/// `MESSAGE_DELETE_BULK`.
#[derive(Deserialize)]
struct MessageDeleteBulkPayload {
    ids: Vec<Id<MessageMarker>>,
    channel_id: Id<ChannelMarker>,
}

/// `MESSAGE_REACTION_ADD`, `MESSAGE_REACTION_REMOVE`, and the two clears,
/// which share a shape: the clears just leave out who reacted, and clearing
/// all of a message's reactions leaves out the emoji too.
#[derive(Deserialize)]
struct ReactionPayload {
    channel_id: Id<ChannelMarker>,
    message_id: Id<MessageMarker>,
    #[serde(default)]
    user_id: Option<Id<UserMarker>>,
    #[serde(default)]
    emoji: Option<twilight_model::channel::message::EmojiReactionType>,
}

#[derive(Deserialize)]
struct ReadyUser {
    id: Id<UserMarker>,
}

/// Deserializes a list item by item, dropping the items that don't fit `T`
/// instead of failing the whole payload. The items are borrowed from the frame
/// first, so the skipping costs no copies.
fn skip_invalid<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let items = Vec::<&'de RawValue>::deserialize(deserializer)?;
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_str(item.get()).ok())
        .collect())
}

fn skip_invalid_opt<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    skip_invalid(deserializer).map(Some)
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
                let mut voice_states = Vec::new();
                let mut roles = Vec::new();
                let mut merged = ready.merged_members.into_iter();
                for guild in ready.guilds {
                    let merged = merged.next().unwrap_or_default();
                    voice_states.extend(convert_guild_voice_states(
                        guild.id,
                        guild.voice_states,
                        guild.members.iter().chain(&merged),
                    ));
                    roles.extend(member_roles(guild.id, merged));
                    roles.extend(guild_emojis(guild.id, guild.emojis));
                    roles.extend(guild_roles(guild.id, guild.roles, guild.members));
                }
                // `Ready` goes first: the member roles that follow are only
                // kept for the signed-in user, whom it names.
                let ready = GatewayEvent::Ready {
                    user_id: ready.user.id,
                    voice_states,
                };
                std::iter::once(ready).chain(roles).collect()
            })
            .unwrap_or_default(),
        "MESSAGE_CREATE" => serde_json::from_str::<twilight_model::channel::Message>(data)
            .map(|message| {
                // twilight's message model has no nonce, so it's read apart.
                let nonce = serde_json::from_str::<NoncePayload>(data)
                    .ok()
                    .and_then(|payload| payload.nonce);
                vec![GatewayEvent::Message(IncomingMessage {
                    channel_id: message.channel_id,
                    message: convert_message(message),
                    nonce,
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
                    nonce: None,
                })]
            })
            .unwrap_or_default(),
        "MESSAGE_DELETE" => serde_json::from_str::<MessageDeletePayload>(data)
            .map(|delete| {
                vec![GatewayEvent::MessageDelete {
                    channel_id: delete.channel_id,
                    message_ids: vec![delete.id],
                }]
            })
            .unwrap_or_default(),
        "MESSAGE_DELETE_BULK" => serde_json::from_str::<MessageDeleteBulkPayload>(data)
            .map(|delete| {
                vec![GatewayEvent::MessageDelete {
                    channel_id: delete.channel_id,
                    message_ids: delete.ids,
                }]
            })
            .unwrap_or_default(),
        "MESSAGE_REACTION_ADD"
        | "MESSAGE_REACTION_REMOVE"
        | "MESSAGE_REACTION_REMOVE_EMOJI"
        | "MESSAGE_REACTION_REMOVE_ALL" => serde_json::from_str::<ReactionPayload>(data)
            .ok()
            .and_then(|payload| {
                let emoji = payload.emoji.map(convert_reaction_emoji);
                let change = match name {
                    "MESSAGE_REACTION_ADD" => ReactionChange::Add {
                        user_id: payload.user_id?,
                        emoji: emoji?,
                    },
                    "MESSAGE_REACTION_REMOVE" => ReactionChange::Remove {
                        user_id: payload.user_id?,
                        emoji: emoji?,
                    },
                    "MESSAGE_REACTION_REMOVE_EMOJI" => ReactionChange::RemoveEmoji(emoji?),
                    _ => ReactionChange::RemoveAll,
                };
                Some(vec![GatewayEvent::Reaction {
                    channel_id: payload.channel_id,
                    message_id: payload.message_id,
                    change,
                }])
            })
            .unwrap_or_default(),
        "VOICE_STATE_UPDATE" => serde_json::from_str::<RawVoiceState>(data)
            .map(|state| vec![GatewayEvent::VoiceState(convert_voice_state(state))])
            .unwrap_or_default(),
        "VOICE_SERVER_UPDATE" => serde_json::from_str::<RawVoiceServer>(data)
            .map(|server| vec![GatewayEvent::VoiceServer(convert_voice_server(server))])
            .unwrap_or_default(),
        "GUILD_CREATE" => serde_json::from_str::<GatewayGuild>(data)
            .map(|guild| {
                let voice = GatewayEvent::GuildVoiceStates {
                    guild_id: guild.id,
                    states: convert_guild_voice_states(
                        guild.id,
                        guild.voice_states,
                        &guild.members,
                    ),
                };
                std::iter::once(voice)
                    .chain(guild_emojis(guild.id, guild.emojis))
                    .chain(guild_roles(guild.id, guild.roles, guild.members))
                    .collect()
            })
            .unwrap_or_default(),
        "GUILD_DELETE" => serde_json::from_str::<GuildDeletePayload>(data)
            .map(|guild| {
                vec![GatewayEvent::GuildVoiceStates {
                    guild_id: guild.id,
                    states: Vec::new(),
                }]
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
        "GUILD_EMOJIS_UPDATE" => serde_json::from_str::<EmojisUpdatePayload>(data)
            .map(|update| {
                guild_emojis(update.guild_id, Some(update.emojis))
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default(),
        "GUILD_MEMBER_UPDATE" => serde_json::from_str::<MemberUpdatePayload>(data)
            .map(|update| member_roles(update.guild_id, vec![update.member]).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A guild's emoji as an event, unless the guild arrived without its list.
fn guild_emojis(guild_id: Id<GuildMarker>, emojis: Option<Vec<RawEmoji>>) -> Option<GatewayEvent> {
    Some(GatewayEvent::GuildEmojis {
        guild_id,
        emojis: emojis?.into_iter().map(convert_guild_emoji).collect(),
    })
}

/// Each member's roles as an event, skipping any that arrived without a user.
fn member_roles(
    guild_id: Id<GuildMarker>,
    members: Vec<RawMember>,
) -> impl Iterator<Item = GatewayEvent> {
    members.into_iter().filter_map(move |member| {
        let user_id = member.user_id()?;
        Some(GatewayEvent::MemberRoles {
            guild_id,
            user_id,
            roles: member.roles,
        })
    })
}

/// A guild's roles and its members' as events. An unavailable guild in
/// `READY` arrives with neither, and yields nothing.
fn guild_roles(
    guild_id: Id<GuildMarker>,
    roles: Vec<RawRole>,
    members: Vec<RawMember>,
) -> impl Iterator<Item = GatewayEvent> {
    let roles_event = (!roles.is_empty()).then(|| GatewayEvent::GuildRoles {
        guild_id,
        roles: roles.into_iter().map(convert_role).collect(),
    });
    roles_event
        .into_iter()
        .chain(member_roles(guild_id, members))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch_json(name: &str, json: &str) -> Vec<GatewayEvent> {
        dispatch(name, &RawValue::from_string(json.to_owned()).unwrap())
    }

    fn ready_voice_states(events: &[GatewayEvent]) -> &[VoiceUserState] {
        match events.first() {
            Some(GatewayEvent::Ready { voice_states, .. }) => voice_states,
            _ => panic!("READY should lead with a Ready event"),
        }
    }

    #[test]
    fn ready_names_voice_states_from_guild_members() {
        let events = dispatch_json(
            "READY",
            r#"{
                "user": {"id": "1"},
                "guilds": [{
                    "id": "10",
                    "voice_states": [
                        {"user_id": "2", "channel_id": "100", "self_mute": true},
                        {"user_id": "3", "channel_id": "100"}
                    ],
                    "members": [{
                        "nick": "Nick",
                        "user": {"id": "2", "username": "user2", "global_name": "Global"}
                    }]
                }]
            }"#,
        );

        let states = ready_voice_states(&events);
        assert_eq!(states.len(), 2);
        assert!(
            states
                .iter()
                .all(|state| state.guild_id == Some(Id::new(10)))
        );
        let named = states.iter().find(|s| s.user_id == Id::new(2)).unwrap();
        assert_eq!(named.name.as_deref(), Some("Nick"));
        assert!(named.self_mute);
        // Not among the members: left for the app to name.
        let unnamed = states.iter().find(|s| s.user_id == Id::new(3)).unwrap();
        assert_eq!(unnamed.name, None);
    }

    #[test]
    fn ready_names_voice_states_from_merged_members() {
        let events = dispatch_json(
            "READY",
            r#"{
                "user": {"id": "1"},
                "guilds": [{
                    "id": "10",
                    "voice_states": [{"user_id": "2", "channel_id": "100"}]
                }],
                "merged_members": [[{"user_id": "2", "nick": "Merged"}]]
            }"#,
        );

        let states = ready_voice_states(&events);
        assert_eq!(states[0].name.as_deref(), Some("Merged"));
    }

    #[test]
    fn ready_skips_a_malformed_voice_state_and_keeps_the_rest() {
        let events = dispatch_json(
            "READY",
            r#"{
                "user": {"id": "1"},
                "guilds": [{
                    "id": "10",
                    "voice_states": [
                        {"channel_id": "100"},
                        {"user_id": "2", "channel_id": "100"}
                    ],
                    "roles": [{"id": "20", "name": "role"}]
                }]
            }"#,
        );

        assert_eq!(ready_voice_states(&events).len(), 1);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, GatewayEvent::GuildRoles { .. }))
        );
    }

    #[test]
    fn guild_create_replaces_the_guilds_voice_states() {
        let events = dispatch_json(
            "GUILD_CREATE",
            r#"{
                "id": "10",
                "voice_states": [{"user_id": "2", "channel_id": "100"}],
                "members": [{
                    "nick": "",
                    "user": {"id": "2", "username": "user2", "global_name": "Global"}
                }]
            }"#,
        );

        let Some(GatewayEvent::GuildVoiceStates { guild_id, states }) = events.first() else {
            panic!("GUILD_CREATE should yield the guild's voice states");
        };
        assert_eq!(*guild_id, Id::new(10));
        // A cleared nickname falls through to the global name.
        assert_eq!(states[0].name.as_deref(), Some("Global"));
    }

    #[test]
    fn guild_emojis_arrive_with_the_guild_and_their_updates() {
        let events = dispatch_json(
            "GUILD_CREATE",
            r#"{
                "id": "10",
                "emojis": [
                    {"id": "5", "name": "wave", "animated": true, "roles": ["20"]},
                    {"id": null, "name": "broken"}
                ]
            }"#,
        );
        let emojis = events.iter().find_map(|event| match event {
            GatewayEvent::GuildEmojis { emojis, .. } => Some(emojis),
            _ => None,
        });
        let [emoji] = emojis
            .expect("GUILD_CREATE should carry its emoji")
            .as_slice()
        else {
            panic!("the malformed emoji should be skipped");
        };
        assert!(emoji.animated && emoji.available);
        assert_eq!(emoji.roles, [Id::new(20)]);

        let events = dispatch_json("GUILD_EMOJIS_UPDATE", r#"{"guild_id": "10", "emojis": []}"#);
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::GuildEmojis { emojis, .. }] if emojis.is_empty()
        ));
    }

    #[test]
    fn guild_delete_clears_the_guilds_voice_states() {
        let events = dispatch_json("GUILD_DELETE", r#"{"id": "10", "unavailable": true}"#);

        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::GuildVoiceStates { guild_id, states }]
                if *guild_id == Id::new(10) && states.is_empty()
        ));
    }

    #[test]
    fn voice_state_update_reads_its_own_member() {
        let events = dispatch_json(
            "VOICE_STATE_UPDATE",
            r#"{
                "user_id": "2",
                "guild_id": "10",
                "channel_id": "100",
                "session_id": "abc",
                "member": {"user": {"id": "2", "username": "user2", "avatar": "hash"}}
            }"#,
        );

        let [GatewayEvent::VoiceState(state)] = events.as_slice() else {
            panic!("VOICE_STATE_UPDATE should yield one voice state");
        };
        assert_eq!(state.guild_id, Some(Id::new(10)));
        assert_eq!(state.name.as_deref(), Some("user2"));
        assert!(state.avatar_url.is_some());
    }

    #[test]
    fn message_deletes_arrive_singly_and_in_bulk() {
        let events = dispatch_json("MESSAGE_DELETE", r#"{"id": "5", "channel_id": "100"}"#);
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::MessageDelete { channel_id, message_ids }]
                if *channel_id == Id::new(100) && message_ids == &[Id::new(5)]
        ));

        let events = dispatch_json(
            "MESSAGE_DELETE_BULK",
            r#"{"ids": ["5", "6"], "channel_id": "100", "guild_id": "10"}"#,
        );
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::MessageDelete { message_ids, .. }] if message_ids.len() == 2
        ));
    }

    #[test]
    fn reactions_read_unicode_and_custom_emoji() {
        let events = dispatch_json(
            "MESSAGE_REACTION_ADD",
            r#"{
                "user_id": "2", "channel_id": "100", "message_id": "5",
                "emoji": {"id": null, "name": "👍"}, "burst": false, "type": 0
            }"#,
        );
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::Reaction {
                change: ReactionChange::Add { user_id, emoji: ReactionEmoji::Unicode(name) },
                ..
            }] if *user_id == Id::new(2) && name == "👍"
        ));

        let events = dispatch_json(
            "MESSAGE_REACTION_REMOVE_EMOJI",
            r#"{
                "channel_id": "100", "message_id": "5",
                "emoji": {"id": "7", "name": "wave", "animated": true}
            }"#,
        );
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::Reaction {
                change: ReactionChange::RemoveEmoji(ReactionEmoji::Custom { animated: true, .. }),
                ..
            }]
        ));

        let events = dispatch_json(
            "MESSAGE_REACTION_REMOVE_ALL",
            r#"{"channel_id": "100", "message_id": "5"}"#,
        );
        assert!(matches!(
            events.as_slice(),
            [GatewayEvent::Reaction {
                change: ReactionChange::RemoveAll,
                ..
            }]
        ));

        // An add without its reactor is malformed, not a clear.
        let events = dispatch_json(
            "MESSAGE_REACTION_ADD",
            r#"{"channel_id": "100", "message_id": "5", "emoji": {"id": null, "name": "👍"}}"#,
        );
        assert!(events.is_empty());
    }
}
