//! Turning dispatches into [`GatewayEvent`]s.

use serde_json::value::RawValue;
use twilight_model::id::{
    Id,
    marker::{ChannelMarker, GuildMarker, UserMarker},
};
use twilight_model::util::Timestamp;

use crate::discord::model::{
    RawEmoji, RawGuildChannel, RawGuildSettings, RawMember, RawMemberListUpdate, RawRole,
    RawVoiceServer, RawVoiceState, StreamServerInfo, author_member, convert_call_voice_states,
    convert_channel_unreads, convert_guild_channels, convert_guild_emoji,
    convert_guild_voice_states, convert_member_list_update, convert_message, convert_mute_settings,
    convert_reaction_emoji, convert_read_states, convert_role, convert_stream_server,
    convert_voice_server, convert_voice_state, parse_user_settings,
};

use super::payloads::*;
use super::{GatewayEvent, IncomingMessage, ReactionChange};

/// Turns one dispatch into the events the app acts on. `READY` and
/// `GUILD_CREATE` fan out into several.
///
/// `self_id` is the signed-in user, remembered from `READY` for the
/// dispatches that need it and don't say.
pub(super) fn dispatch(
    name: &str,
    data: &RawValue,
    self_id: &mut Option<Id<UserMarker>>,
) -> Vec<GatewayEvent> {
    let data = data.get();
    match name {
        "READY" => serde_json::from_str::<ReadyPayload>(data)
            .map(|ready| {
                *self_id = Some(ready.user.id);
                let mut voice_states = Vec::new();
                let mut roles = Vec::new();
                let mut unreads = vec![
                    GatewayEvent::ReadStates(convert_read_states(ready.read_state)),
                    // The mention counts ride the same read_state list the
                    // conversion above parses for acks, which keeps none of
                    // them; one more scan of READY per session is nothing
                    // next to the fan-out it feeds.
                    GatewayEvent::MentionCounts(mention_counts(data)),
                    GatewayEvent::MuteSettings(
                        ready
                            .user_guild_settings
                            .into_iter()
                            .map(convert_mute_settings)
                            .collect(),
                    ),
                    GatewayEvent::LatestMessages(convert_channel_unreads(ready.private_channels)),
                ];
                let mut presences = ready.presences;
                presences.extend(
                    ready
                        .merged_presences
                        .into_iter()
                        .flat_map(MergedPresences::into_presences),
                );
                let mut merged = ready.merged_members.into_iter();
                for mut guild in ready.guilds {
                    presences.append(&mut guild.presences);
                    let merged = merged.next().unwrap_or_default();
                    let owner_id = guild.owner_id();
                    unreads.extend(guild_channels(
                        guild.id,
                        ready.user.id,
                        owner_id,
                        guild.joined_at,
                        &guild.roles,
                        guild.members.iter().chain(&merged),
                        guild.channels,
                    ));
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
                let presences = GatewayEvent::Presences(convert_presences(presences));
                std::iter::once(ready)
                    .chain(unreads)
                    .chain(std::iter::once(presences))
                    .chain(roles)
                    .collect()
            })
            .unwrap_or_default(),
        "MESSAGE_CREATE" => serde_json::from_str::<twilight_model::channel::Message>(data)
            .map(|message| {
                // twilight's message model has no nonce, so it's read apart.
                let nonce = serde_json::from_str::<NoncePayload>(data)
                    .ok()
                    .and_then(|payload| payload.nonce);
                let roles = author_roles(&message);
                std::iter::once(GatewayEvent::Message(IncomingMessage {
                    channel_id: message.channel_id,
                    guild_id: message.guild_id,
                    message: convert_message(message),
                    nonce,
                }))
                .chain(roles)
                .collect()
            })
            .unwrap_or_default(),
        // twilight models this dispatch as a partial message, because Discord
        // hasn't always sent the whole thing. Parsing it as a full one takes
        // the updates that do and skips any that don't.
        "MESSAGE_UPDATE" => serde_json::from_str::<twilight_model::channel::Message>(data)
            .map(|message| {
                let roles = author_roles(&message);
                std::iter::once(GatewayEvent::MessageUpdate(IncomingMessage {
                    channel_id: message.channel_id,
                    guild_id: message.guild_id,
                    message: convert_message(message),
                    nonce: None,
                }))
                .chain(roles)
                .collect()
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
        "TYPING_START" => serde_json::from_str::<TypingStartPayload>(data)
            .map(|typing| {
                vec![GatewayEvent::Typing {
                    channel_id: typing.channel_id,
                    user_id: typing.user_id,
                    name: typing.member.and_then(|member| member.display_name()),
                }]
            })
            .unwrap_or_default(),
        "VOICE_STATE_UPDATE" => serde_json::from_str::<RawVoiceState>(data)
            .map(|state| vec![GatewayEvent::VoiceState(convert_voice_state(state))])
            .unwrap_or_default(),
        "VOICE_SERVER_UPDATE" => serde_json::from_str::<RawVoiceServer>(data)
            .map(|server| vec![GatewayEvent::VoiceServer(convert_voice_server(server))])
            .unwrap_or_default(),
        "CALL_CREATE" => serde_json::from_str::<CallPayload>(data)
            .map(|call| {
                vec![GatewayEvent::CallCreate {
                    channel_id: call.channel_id,
                    ringing: call.ringing,
                    voice_states: convert_call_voice_states(call.channel_id, call.voice_states),
                }]
            })
            .unwrap_or_default(),
        "CALL_UPDATE" => serde_json::from_str::<CallPayload>(data)
            .map(|call| {
                vec![GatewayEvent::CallUpdate {
                    channel_id: call.channel_id,
                    ringing: call.ringing,
                }]
            })
            .unwrap_or_default(),
        "CALL_DELETE" => serde_json::from_str::<CallDeletePayload>(data)
            .map(|call| {
                vec![GatewayEvent::CallDelete {
                    channel_id: call.channel_id,
                }]
            })
            .unwrap_or_default(),
        "STREAM_CREATE" => serde_json::from_str::<StreamCreatePayload>(data)
            .map(|stream| {
                vec![GatewayEvent::StreamCreate {
                    stream_key: stream.stream_key,
                    rtc_server_id: stream.rtc_server_id,
                }]
            })
            .unwrap_or_default(),
        "STREAM_SERVER_UPDATE" => serde_json::from_str::<StreamServerInfo>(data)
            .map(|server| vec![GatewayEvent::StreamServer(convert_stream_server(server))])
            .unwrap_or_default(),
        "STREAM_DELETE" => serde_json::from_str::<StreamDeletePayload>(data)
            .map(|stream| {
                vec![GatewayEvent::StreamDelete {
                    stream_key: stream.stream_key,
                    reason: stream.reason,
                }]
            })
            .unwrap_or_default(),
        "GUILD_CREATE" => serde_json::from_str::<GatewayGuild>(data)
            .map(|guild| {
                let owner_id = guild.owner_id();
                let channels = match *self_id {
                    Some(self_id) => guild_channels(
                        guild.id,
                        self_id,
                        owner_id,
                        guild.joined_at,
                        &guild.roles,
                        &guild.members,
                        guild.channels,
                    ),
                    None => Vec::new(),
                };
                let voice = GatewayEvent::GuildVoiceStates {
                    guild_id: guild.id,
                    states: convert_guild_voice_states(
                        guild.id,
                        guild.voice_states,
                        &guild.members,
                    ),
                };
                let presences = GatewayEvent::Presences(convert_presences(guild.presences));
                std::iter::once(voice)
                    .chain(std::iter::once(presences))
                    .chain(channels)
                    .chain(guild_emojis(guild.id, guild.emojis))
                    .chain(guild_roles(guild.id, guild.roles, guild.members))
                    .collect()
            })
            .unwrap_or_default(),
        // Working the change out here would take every guild's roles and the
        // user's member kept on this side; the list is refetched instead, the
        // next time it's looked at.
        "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "CHANNEL_DELETE" => {
            serde_json::from_str::<ChannelChangePayload>(data)
                .ok()
                .and_then(|channel| channel.guild_id)
                .map(|guild_id| vec![GatewayEvent::GuildChannelsChanged { guild_id }])
                .unwrap_or_default()
        }
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
            .map(|update| {
                // A changed nickname renames them in messages already drawn.
                let named = update
                    .member
                    .guild_member()
                    .map(|member| GatewayEvent::Members {
                        guild_id: update.guild_id,
                        members: vec![member],
                    });
                member_roles(update.guild_id, vec![update.member])
                    .chain(named)
                    .collect()
            })
            .unwrap_or_default(),
        "GUILD_MEMBERS_CHUNK" => serde_json::from_str::<MembersChunkPayload>(data)
            .map(|chunk| {
                let members: Vec<_> = chunk
                    .members
                    .iter()
                    .filter_map(RawMember::guild_member)
                    .collect();
                let named = (!members.is_empty()).then_some(GatewayEvent::Members {
                    guild_id: chunk.guild_id,
                    members,
                });
                member_roles(chunk.guild_id, chunk.members)
                    .chain(named)
                    .collect()
            })
            .unwrap_or_default(),
        "GUILD_MEMBER_LIST_UPDATE" => serde_json::from_str::<RawMemberListUpdate>(data)
            .map(|update| vec![GatewayEvent::MemberList(convert_member_list_update(update))])
            .unwrap_or_default(),
        "READY_SUPPLEMENTAL" => serde_json::from_str::<ReadySupplementalPayload>(data)
            .ok()
            .and_then(|ready| ready.merged_presences)
            .map(|merged| {
                vec![GatewayEvent::Presences(convert_presences(
                    merged.into_presences(),
                ))]
            })
            .unwrap_or_default(),
        "PRESENCE_UPDATE" => serde_json::from_str::<RawPresence>(data)
            .map(|presence| vec![GatewayEvent::Presences(convert_presences([presence]))])
            .unwrap_or_default(),
        "USER_SETTINGS_PROTO_UPDATE" => serde_json::from_str::<SettingsProtoUpdatePayload>(data)
            .ok()
            .filter(|update| update.settings.kind == 1)
            .and_then(|update| parse_user_settings(&update.settings.proto).ok()?.status)
            .map(|status| vec![GatewayEvent::StatusSettings(status)])
            .unwrap_or_default(),
        "MESSAGE_ACK" => serde_json::from_str::<MessageAckPayload>(data)
            .map(|ack| {
                vec![GatewayEvent::MessageAck {
                    channel_id: ack.channel_id,
                    message_id: ack.message_id,
                    manual: ack.manual,
                }]
            })
            .unwrap_or_default(),
        "CHANNEL_UNREAD_UPDATE" | "PASSIVE_UPDATE_V1" | "PASSIVE_UPDATE_V2" => {
            serde_json::from_str::<ChannelUnreadsPayload>(data)
                .map(|update| convert_channel_unreads(update.channel_unread_updates))
                .ok()
                .filter(|channels| !channels.is_empty())
                .map(|channels| vec![GatewayEvent::LatestMessages(channels)])
                .unwrap_or_default()
        }
        "USER_GUILD_SETTINGS_UPDATE" => serde_json::from_str::<RawGuildSettings>(data)
            .map(|settings| {
                vec![GatewayEvent::MuteSettings(vec![convert_mute_settings(
                    settings,
                )])]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `READY`'s read states, of which only channels' unread mention counts are
/// kept. A payload that doesn't fit means no badges, like any other dispatch
/// the app reads leniently.
fn mention_counts(data: &str) -> Vec<(Id<ChannelMarker>, u32)> {
    serde_json::from_str::<MentionStatesPayload>(data)
        .ok()
        .map(MentionStatesPayload::into_mention_counts)
        .unwrap_or_default()
}

/// A guild's channels as events — the sidebar's list and its unread mark —
/// unless it arrived without any: an unavailable guild, which says nothing
/// about them. `members` is searched for the signed-in user's own, whose
/// roles decide what they see.
fn guild_channels<'a>(
    guild_id: Id<GuildMarker>,
    self_id: Id<UserMarker>,
    owner_id: Option<Id<UserMarker>>,
    joined_at: Option<Timestamp>,
    roles: &[RawRole],
    members: impl IntoIterator<Item = &'a RawMember>,
    channels: Vec<RawGuildChannel>,
) -> Vec<GatewayEvent> {
    if channels.is_empty() {
        return Vec::new();
    }
    let member_roles = members
        .into_iter()
        .find(|member| member.user_id() == Some(self_id))
        .map(|member| member.roles.as_slice());
    let (unreads, channels) = convert_guild_channels(
        guild_id,
        self_id,
        owner_id,
        joined_at,
        roles,
        member_roles,
        channels,
    );
    vec![
        GatewayEvent::GuildUnreads(unreads),
        GatewayEvent::GuildChannels { guild_id, channels },
    ]
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

/// The roles and name of a guild message's author, from the member Discord
/// sends along with it. Webhooks aren't members, so their messages have none.
/// The name is kept because fetched history carries no members, and without
/// it the same author's older messages would show their global name.
fn author_roles(message: &twilight_model::channel::Message) -> Vec<GatewayEvent> {
    let (Some(guild_id), Some(member)) = (message.guild_id, message.member.as_ref()) else {
        return Vec::new();
    };
    let roles = GatewayEvent::MemberRoles {
        guild_id,
        user_id: message.author.id,
        roles: member.roles.clone(),
    };
    let named = author_member(message).map(|member| GatewayEvent::Members {
        guild_id,
        members: vec![member],
    });
    std::iter::once(roles).chain(named).collect()
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
