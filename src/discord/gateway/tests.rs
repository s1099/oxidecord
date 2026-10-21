use serde_json::value::RawValue;
use twilight_model::id::Id;

use super::dispatch::dispatch;
use super::*;
use crate::discord::{MemberGroupKind, MemberListGroup, MemberListItem, MemberListOp};

fn dispatch_json(name: &str, json: &str) -> Vec<GatewayEvent> {
    dispatch(
        name,
        &RawValue::from_string(json.to_owned()).unwrap(),
        &mut Some(Id::new(1)),
    )
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
fn call_create_puts_its_voice_states_in_the_call() {
    let events = dispatch_json(
        "CALL_CREATE",
        r#"{
            "channel_id": "200",
            "message_id": "5",
            "region": "india",
            "ringing": ["1"],
            "voice_states": [{"user_id": "2", "session_id": "abc", "self_mute": true}]
        }"#,
    );

    let [
        GatewayEvent::CallCreate {
            channel_id,
            ringing,
            voice_states,
        },
    ] = events.as_slice()
    else {
        panic!("CALL_CREATE should yield one call");
    };
    assert_eq!(*channel_id, Id::new(200));
    assert_eq!(ringing, &[Id::new(1)]);
    let [state] = voice_states.as_slice() else {
        panic!("the call should carry its one voice state");
    };
    assert_eq!(state.channel_id, Some(Id::new(200)));
    assert!(state.self_mute);
}

#[test]
fn ready_carries_read_states_and_the_channels_the_user_can_see() {
    let events = dispatch_json(
        "READY",
        r#"{
            "user": {"id": "1"},
            "read_state": [{"id": "100", "last_message_id": "40"}],
            "private_channels": [{"id": "200", "type": 1, "last_message_id": "9"}],
            "guilds": [{
                "id": "10",
                "joined_at": "2020-01-01T00:00:00+00:00",
                "roles": [
                    {"id": "10", "name": "@everyone", "permissions": "0"},
                    {"id": "20", "name": "member", "permissions": "1024"}
                ],
                "channels": [
                    {"id": "100", "type": 0, "last_message_id": "50"}
                ]
            }],
            "merged_members": [[{"user_id": "1", "roles": ["20"]}]]
        }"#,
    );

    assert!(events.iter().any(|event| matches!(
        event,
        GatewayEvent::ReadStates(states) if states.len() == 1
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        GatewayEvent::LatestMessages(channels)
            if channels[0].channel_id == Id::new(200)
    )));
    // Visible through the member's role, not `@everyone`.
    let Some(GatewayEvent::GuildUnreads(guild)) = events
        .iter()
        .find(|event| matches!(event, GatewayEvent::GuildUnreads(_)))
    else {
        panic!("READY should carry the guild's channels");
    };
    assert_eq!(guild.channels.len(), 1);
    assert!(guild.joined.is_some());
    // And the sidebar's list comes with it, so opening the guild needs no
    // request.
    assert!(events.iter().any(|event| matches!(
        event,
        GatewayEvent::GuildChannels { guild_id, channels }
            if *guild_id == Id::new(10) && channels.len() == 1
    )));
}

#[test]
fn channel_changes_mark_their_guild_stale() {
    let events = dispatch_json(
        "CHANNEL_UPDATE",
        r#"{"id": "100", "type": 0, "guild_id": "10", "name": "general"}"#,
    );
    assert!(matches!(
        events.as_slice(),
        [GatewayEvent::GuildChannelsChanged { guild_id }] if *guild_id == Id::new(10)
    ));
    // A DM's has no guild to mark.
    assert!(dispatch_json("CHANNEL_CREATE", r#"{"id": "200", "type": 1}"#).is_empty());
}

#[test]
fn presences_arrive_from_ready_supplemental_and_updates() {
    let events = dispatch_json(
        "READY_SUPPLEMENTAL",
        r#"{
            "merged_presences": {
                "friends": [{"user_id": "2", "status": "idle"}],
                "guilds": [[{"user_id": "3", "status": "dnd"}, {"user_id": "4", "status": "streaming"}]]
            }
        }"#,
    );
    let [GatewayEvent::Presences(presences)] = events.as_slice() else {
        panic!("READY_SUPPLEMENTAL should yield its presences");
    };
    // An unknown status is skipped rather than guessed at.
    assert_eq!(
        presences.as_slice(),
        [
            (Id::new(2), PresenceStatus::Idle),
            (Id::new(3), PresenceStatus::DoNotDisturb)
        ]
    );

    let events = dispatch_json(
        "PRESENCE_UPDATE",
        r#"{"user": {"id": "2"}, "status": "offline", "activities": [], "guild_id": "10"}"#,
    );
    assert!(matches!(
        events.as_slice(),
        [GatewayEvent::Presences(presences)]
            if presences.as_slice() == [(Id::new(2), PresenceStatus::Invisible)]
    ));
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

#[test]
fn member_roles_arrive_from_member_chunks_and_messages() {
    let events = dispatch_json(
        "GUILD_MEMBERS_CHUNK",
        r#"{
            "guild_id": "10",
            "members": [{"user": {"id": "2"}, "roles": ["20", "30"]}],
            "chunk_index": 0,
            "chunk_count": 1,
            "not_found": ["3"]
        }"#,
    );
    let [
        GatewayEvent::MemberRoles {
            guild_id,
            user_id,
            roles,
        },
    ] = events.as_slice()
    else {
        panic!("GUILD_MEMBERS_CHUNK should yield each member's roles");
    };
    assert_eq!((*guild_id, *user_id), (Id::new(10), Id::new(2)));
    assert_eq!(roles, &[Id::new(20), Id::new(30)]);

    let events = dispatch_json(
        "MESSAGE_CREATE",
        r#"{
            "id": "500",
            "channel_id": "100",
            "guild_id": "10",
            "author": {"id": "4", "username": "user4", "discriminator": "0", "avatar": null},
            "member": {"roles": ["20"], "joined_at": "2020-01-01T00:00:00+00:00", "deaf": false, "mute": false, "flags": 0},
            "content": "hi",
            "timestamp": "2020-01-01T00:00:00+00:00",
            "edited_timestamp": null,
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0
        }"#,
    );
    assert!(events.iter().any(|event| matches!(
        event,
        GatewayEvent::MemberRoles { user_id, roles, .. }
            if *user_id == Id::new(4) && roles == &[Id::new(20)]
    )));
}

#[test]
fn a_message_names_its_author_by_their_nickname() {
    let events = dispatch_json(
        "MESSAGE_CREATE",
        r#"{
            "id": "500",
            "channel_id": "100",
            "guild_id": "10",
            "author": {"id": "4", "username": "user4", "global_name": "Four", "discriminator": "0", "avatar": null},
            "member": {"nick": "Quad", "roles": [], "joined_at": "2020-01-01T00:00:00+00:00", "deaf": false, "mute": false, "flags": 0},
            "content": "hi",
            "timestamp": "2020-01-01T00:00:00+00:00",
            "edited_timestamp": null,
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0
        }"#,
    );
    // Kept apart from the message, so its fetched history, which carries no
    // members, can be named the same way.
    let named = events.iter().find_map(|event| match event {
        GatewayEvent::Members { guild_id, members } => Some((*guild_id, members)),
        _ => None,
    });
    let Some((guild_id, [member])) = named.map(|(id, members)| (id, members.as_slice())) else {
        panic!("MESSAGE_CREATE should name its author as a guild member");
    };
    assert_eq!(guild_id, Id::new(10));
    assert_eq!(member.user_id, Id::new(4));
    assert_eq!(member.name, "Quad");
    assert_eq!(member.username.as_deref(), Some("user4"));
}

#[test]
fn member_list_update_reads_groups_members_and_edits() {
    let events = dispatch_json(
        "GUILD_MEMBER_LIST_UPDATE",
        r#"{
            "guild_id": "10",
            "id": "everyone",
            "member_count": 3,
            "online_count": 2,
            "groups": [{"id": "20", "count": 1}, {"id": "online", "count": 1}, {"id": "offline", "count": 1}],
            "ops": [
                {"op": "SYNC", "range": [0, 99], "items": [
                    {"group": {"id": "20"}},
                    {"member": {
                        "user": {"id": "2", "username": "user2", "global_name": "Two", "avatar": null, "bot": true},
                        "nick": null,
                        "roles": ["20"],
                        "presence": {"status": "dnd", "activities": [
                            {"type": 0, "name": "Chess"},
                            {"type": 4, "name": "Custom Status", "state": "busy"}
                        ]}
                    }},
                    {"group": {"id": "offline"}},
                    {"member": {"user": {"id": "3", "username": "user3"}, "roles": []}}
                ]},
                {"op": "UPDATE", "index": 3, "item": {"member": {
                    "user": {"id": "3", "username": "user3"}, "roles": [],
                    "presence": {"status": "online", "activities": [{"type": 2, "name": "Spotify"}]}
                }}},
                {"op": "DELETE", "index": 1},
                {"op": "INVALIDATE", "range": [100, 199]}
            ]
        }"#,
    );
    let [GatewayEvent::MemberList(update)] = events.as_slice() else {
        panic!("GUILD_MEMBER_LIST_UPDATE should yield the update");
    };
    assert_eq!(update.guild_id, Id::new(10));
    assert_eq!(update.list_id, "everyone");
    assert!(matches!(
        update.groups.as_slice(),
        [
            MemberListGroup { kind: MemberGroupKind::Role(role), count: 1 },
            MemberListGroup { kind: MemberGroupKind::Online, .. },
            MemberListGroup { kind: MemberGroupKind::Offline, .. },
        ] if *role == Id::new(20)
    ));

    let [
        MemberListOp::Sync { start: 0, items },
        MemberListOp::Update {
            index: 3,
            item: MemberListItem::Member(updated),
        },
        MemberListOp::Delete { index: 1 },
        MemberListOp::Invalidate {
            start: 100,
            end: 199,
        },
    ] = update.ops.as_slice()
    else {
        panic!("each op should be read in order");
    };
    let [
        MemberListItem::Group(_),
        MemberListItem::Member(two),
        MemberListItem::Group(_),
        MemberListItem::Member(three),
    ] = items.as_slice()
    else {
        panic!("the sync should hold two headers and two members");
    };
    assert_eq!(two.name, "Two");
    assert!(two.bot);
    assert_eq!(two.roles, &[Id::new(20)]);
    assert_eq!(two.status, PresenceStatus::DoNotDisturb);
    // The custom status wins over the game.
    assert_eq!(two.activity.as_deref(), Some("busy"));
    // No presence reads as offline.
    assert_eq!(three.status, PresenceStatus::Invisible);
    assert_eq!(updated.activity.as_deref(), Some("Listening to Spotify"));
}

#[test]
fn members_chunk_names_the_members_it_carries() {
    let events = dispatch_json(
        "GUILD_MEMBERS_CHUNK",
        r#"{
            "guild_id": "10",
            "members": [
                {"user": {"id": "2", "username": "two", "global_name": "Two"}, "nick": "", "roles": []},
                {"user": {"id": "3", "username": "three"}, "nick": "Trey", "roles": []}
            ],
            "chunk_index": 0,
            "chunk_count": 1
        }"#,
    );
    let Some(GatewayEvent::Members { guild_id, members }) = events.last() else {
        panic!("GUILD_MEMBERS_CHUNK should end with the members it names");
    };
    assert_eq!(*guild_id, Id::new(10));
    let names: Vec<_> = members
        .iter()
        .map(|member| (member.name.as_str(), member.username.as_deref()))
        .collect();
    // A cleared nickname falls back to the global name.
    assert_eq!(names, [("Two", Some("two")), ("Trey", Some("three"))]);
}

#[test]
fn ready_read_states_carry_channel_mention_counts() {
    let events = dispatch_json(
        "READY",
        r#"{"user":{"id":"1"},"guilds":[],"read_state":[
            {"id":"7","last_message_id":"9","mention_count":3},
            {"id":"8","last_message_id":"2"},
            {"id":"9","last_message_id":"4","mention_count":0},
            {"id":"10","last_acked_id":"1","read_state_type":2,"mention_count":5}
        ]}"#,
    );
    // Only channel read states with a positive count become badges: the
    // channel without one, the zero, and the type-2 guild-event state (whose
    // id isn't a channel) all drop out.
    assert!(events.iter().any(|event| matches!(
        event,
        GatewayEvent::MentionCounts(counts) if *counts == [(Id::new(7), 3)]
    )));
}
