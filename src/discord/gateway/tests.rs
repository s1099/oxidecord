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
fn guild_stickers_arrive_with_the_guild_and_their_updates() {
    let events = dispatch_json(
        "GUILD_CREATE",
        r#"{
            "id": "10",
            "stickers": [
                {"id": "5", "name": "hug", "format_type": 2, "type": 2, "tags": "hug", "guild_id": "10", "available": false},
                {"id": "6", "name": "broken"}
            ]
        }"#,
    );
    let stickers = events.iter().find_map(|event| match event {
        GatewayEvent::GuildStickers { stickers, .. } => Some(stickers),
        _ => None,
    });
    let [sticker] = stickers
        .expect("GUILD_CREATE should carry its stickers")
        .as_slice()
    else {
        panic!("the malformed sticker should be skipped");
    };
    assert_eq!(sticker.item.format, crate::discord::StickerFormat::Apng);
    assert!(!sticker.available);

    let events = dispatch_json(
        "GUILD_STICKERS_UPDATE",
        r#"{"guild_id": "10", "stickers": []}"#,
    );
    assert!(matches!(
        events.as_slice(),
        [GatewayEvent::GuildStickers { stickers, .. }] if stickers.is_empty()
    ));
}

#[test]
fn guild_delete_clears_the_guilds_voice_states_and_activities() {
    let events = dispatch_json("GUILD_DELETE", r#"{"id": "10", "unavailable": true}"#);

    assert!(matches!(
        events.as_slice(),
        [
            GatewayEvent::GuildVoiceStates { guild_id, states },
            GatewayEvent::GuildActivities { instances, .. },
        ] if *guild_id == Id::new(10) && states.is_empty() && instances.is_empty()
    ));
}

#[test]
fn activity_instances_arrive_with_supplemental_and_updates() {
    let events = dispatch_json(
        "READY_SUPPLEMENTAL",
        r#"{"guilds": [{"id": "10", "activity_instances": [{
            "application_id": "7", "composite_instance_id": "i-1-gc-10-20",
            "instance_id": "1", "launch_id": "1",
            "location": {"id": "gc-10-20", "kind": "gc", "channel_id": "20", "guild_id": "10"},
            "participants": [{"user_id": "2", "session_id": "s"}]
        }]}]}"#,
    );
    let [
        GatewayEvent::GuildActivities {
            guild_id,
            instances,
        },
    ] = events.as_slice()
    else {
        panic!("READY_SUPPLEMENTAL should yield the guild's activities");
    };
    assert_eq!(*guild_id, Id::new(10));
    assert_eq!(instances[0].channel_id, Id::new(20));

    let events = dispatch_json(
        "EMBEDDED_ACTIVITY_UPDATE_V2",
        r#"{"application_id": "7", "composite_instance_id": "i-1-pc-30",
            "instance_id": "1", "launch_id": "1",
            "location": {"id": "pc-30", "kind": "pc", "channel_id": "30"},
            "participants": []}"#,
    );
    let [GatewayEvent::ActivityInstance(instance)] = events.as_slice() else {
        panic!("EMBEDDED_ACTIVITY_UPDATE_V2 should yield the instance");
    };
    assert!(instance.guild_id.is_none() && instance.participants.is_empty());
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
fn group_dm_recipients_join_and_leave() {
    let events = dispatch_json(
        "CHANNEL_RECIPIENT_ADD",
        r#"{"channel_id": "200", "user": {"id": "5", "username": "ann", "global_name": "Ann", "avatar": null, "discriminator": "0"}}"#,
    );
    assert!(matches!(
        events.as_slice(),
        [GatewayEvent::DmRecipientAdd { channel_id, member }]
            if *channel_id == Id::new(200) && member.user_id == Id::new(5) && member.name == "Ann"
    ));

    let events = dispatch_json(
        "CHANNEL_RECIPIENT_REMOVE",
        r#"{"channel_id": "200", "user": {"id": "5", "username": "ann"}}"#,
    );
    assert!(matches!(
        events.as_slice(),
        [GatewayEvent::DmRecipientRemove { channel_id, user_id }]
            if *channel_id == Id::new(200) && *user_id == Id::new(5)
    ));
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
fn modal_create_reads_labels_rows_and_text() {
    let events = dispatch_json(
        "INTERACTION_MODAL_CREATE",
        r#"{
            "id": "500", "nonce": "77", "channel_id": "9", "title": "Feedback",
            "custom_id": "form",
            "application": {"id": "42", "name": "Bot", "icon": null},
            "components": [
                {"type": 1, "components": [
                    {"type": 4, "custom_id": "name", "style": 1, "label": "Name", "required": false}
                ]},
                {"type": 10, "content": "Tell us more"},
                {"type": 18, "label": "Pick", "description": "One only", "component":
                    {"type": 3, "custom_id": "pick", "options": [
                        {"label": "A", "value": "a", "default": true},
                        {"label": "B", "value": "b"}
                    ]}
                }
            ]
        }"#,
    );
    let Some(GatewayEvent::Modal(modal)) = events.first() else {
        panic!("expected a modal");
    };
    assert_eq!(modal.nonce, Some(77));
    assert_eq!(modal.title, "Feedback");
    assert_eq!(modal.fields.len(), 3);
    let crate::discord::ModalField::Input(name) = &modal.fields[0] else {
        panic!("expected the text input first");
    };
    assert_eq!(name.label, "Name");
    assert!(!name.in_label);
    assert!(matches!(
        name.kind,
        crate::discord::InputKind::Text {
            required: false,
            paragraph: false,
            ..
        }
    ));
    assert!(
        matches!(&modal.fields[1], crate::discord::ModalField::Text(text) if text == "Tell us more")
    );
    let crate::discord::ModalField::Input(pick) = &modal.fields[2] else {
        panic!("expected the labelled select");
    };
    assert!(pick.in_label);
    assert_eq!(pick.description.as_deref(), Some("One only"));
    let crate::discord::InputKind::Select(select) = &pick.kind else {
        panic!("expected a select");
    };
    assert_eq!(select.options.len(), 2);
    assert!(select.options[0].default);
}
