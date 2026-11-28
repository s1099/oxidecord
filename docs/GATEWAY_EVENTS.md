# Unhandled Gateway Events

The dispatch events Discord sends a user account that `src/discord/gateway/dispatch.rs` doesn't
handle yet, and what the app would use each for. Anything not matched there falls through to
`_ => Vec::new()` and is dropped. Event names and meanings follow
<https://docs.discord.food/topics/gateway-events>; check it for payload shapes before wiring one up.

When an event here gets handled, delete its row. Events Discord never sends to user accounts
(AutoMod rules and executions, `INVITE_DELETE`) and ones that only matter to bots or the
Social SDK aren't listed.

## Connection and session

| Event | Will be used for |
|---|---|
| `RESUMED` | Clearing the offline/reconnecting indicator once a resume succeeds, instead of waiting for the next dispatch |
| `RATE_LIMITED` | Backing off gateway requests Discord throttled (member searches, guild subscriptions) instead of silently getting no answer |
| `SESSIONS_REPLACE` | Your own status across devices: the status shown for yourself and the Sessions and devices settings page |
| `REMOTE_COMMAND` | Answering commands from your other sessions (e.g. "join call on this device", remote voice control) |
| `AUTH_SESSION_CHANGE` | Noticing a logout or token change made elsewhere |
| `USER_REQUIRED_ACTION_UPDATE` | Prompting for phone or email verification Discord now demands, rather than failing requests |

## Messages

| Event | Will be used for |
|---|---|
| `CHANNEL_PINS_UPDATE` | Keeping the pinned-messages popout current, and the "new pins" dot on the header's pin button |
| `CHANNEL_PINS_ACK` | Clearing that dot when the pins were read on another client |
| `MESSAGE_REACTION_ADD_MANY` | Debounced reaction bursts, if the `DEBOUNCE_MESSAGE_REACTIONS` capability is turned on |
| `REACTION_NOTIFICATION_SENT` | Notifying when someone reacts to your message, as Discord's client does |
| `RECENT_MENTION_DELETE` | Removing a mention from the inbox's Mentions tab once it's dismissed elsewhere |
| `LAST_MESSAGES` | Answer to the Request Last Messages opcode: latest message per channel for channels READY didn't cover, without a REST call each |
| `SAVED_MESSAGE_CREATE` / `SAVED_MESSAGE_DELETE` | Saved messages ("Save for later") and keeping them in sync |
| `CONVERSATION_SUMMARY_UPDATE` | Channel conversation summaries, where Discord offers them |

## Channels

| Event | Will be used for |
|---|---|
| `CHANNEL_SYNC` | Answer to Resync Guild Channels: refreshing a guild's channel list over the gateway instead of REST when it went stale |
| `CHANNEL_STATUSES` | Answer to Request Channel Statuses: voice channel statuses for a guild on open |
| `CHANNEL_INFO` | Answer to Request Channel Info: lightweight channel details without fetching the guild |
| `CHANNEL_MEMBER_COUNT_UPDATE` | Member counts on channels and threads |
| `VOICE_CHANNEL_STATUS_UPDATE` | The status text under a voice channel in the sidebar |
| `VOICE_CHANNEL_START_TIME_UPDATE` | "In voice for 12 minutes" timers on voice channels |
| `WEBHOOKS_UPDATE` | Refreshing the channel's integrations settings when webhooks change |
| `STAGE_INSTANCE_CREATE` / `_UPDATE` / `_DELETE` | Stage channels: the live topic, the "live" badge in the sidebar, and closing the stage |

## Guilds and members

| Event | Will be used for |
|---|---|
| `GUILD_MEMBER_ADD` / `GUILD_MEMBER_REMOVE` | Keeping cached members current for mentions and profiles; the member list itself already follows `GUILD_MEMBER_LIST_UPDATE` |
| `GUILD_BAN_ADD` / `GUILD_BAN_REMOVE` | The bans page in server settings, once moderation exists |
| `GUILD_AUDIT_LOG_ENTRY_CREATE` | A live audit log in server settings |
| `GUILD_INTEGRATIONS_UPDATE`, `INTEGRATION_CREATE` / `_UPDATE` / `_DELETE` | The integrations page in server settings |
| `INVITE_CREATE` / `INVITE_UPDATE` | The invites list in server settings |
| `GUILD_JOIN_REQUEST_CREATE` / `_UPDATE` / `_DELETE` | Membership screening and join applications, both applying and reviewing |
| `GUILD_APPLIED_BOOSTS_UPDATE` | Boost count and level perks shown in the server header |
| `GUILD_SCHEDULED_EVENT_CREATE` / `_UPDATE` / `_DELETE` | Scheduled events: the events count in the channel list and the events popout |
| `GUILD_SCHEDULED_EVENT_USER_ADD` / `_REMOVE` | "Interested" counts and your own interest in an event |
| `GUILD_SCHEDULED_EVENT_EXCEPTION_*` | Skipped or moved occurrences of a recurring event |
| `GUILD_SOUNDBOARD_SOUND_CREATE` / `_UPDATE` / `_DELETE`, `GUILD_SOUNDBOARD_SOUNDS_UPDATE` | Keeping the soundboard picker's guild sounds current |
| `SOUNDBOARD_SOUNDS` | Answer to Request Soundboard Sounds: filling the soundboard picker |
| `GUILD_FEATURE_ACK` | Read state for guild features (e.g. the events or onboarding dot) |
| `GUILD_DIRECTORY_ENTRY_*` | Student hub and directory channels |
| `GUILD_POWERUP_ENTITLEMENTS_CREATE` / `_DELETE` | Server boost power-ups unlocked for a guild |
| `APPLICATION_COMMAND_PERMISSIONS_UPDATE` | Hiding slash commands the user lost permission to, without waiting for the command index to be refetched |

## Voice and streams

| Event | Will be used for |
|---|---|
| `STREAM_UPDATE` | A stream's paused state and viewer list on the call stage and stream tile |
| `VOICE_CHANNEL_EFFECT_SEND` | Emoji reactions and soundboard sounds played in the call: drawing them on the tiles and playing the sound |
| `SPEED_TEST_*` | Measuring connection quality before Go Live picks a bitrate |

## Social and users

| Event | Will be used for |
|---|---|
| `USER_UPDATE` | Your own avatar, name, banner and flags changing on another client |
| `USER_NOTE_UPDATE` | The note field on profiles |
| `USER_CONNECTIONS_UPDATE` | The Connections settings page and connections on your own profile |
| `FRIEND_SUGGESTION_CREATE` / `_DELETE` | Friend suggestions on the friends page |
| `ACTIVITY_INVITE_CREATE` | Game and activity invites ("join my game") shown in DMs |
| `GAME_INVITE_CREATE` / `_DELETE` / `_DELETE_MANY` | Game invites from games built on the Social SDK |
| `NOTIFICATION_SETTINGS_UPDATE` | Account-wide notification settings (e.g. flash the taskbar), once those settings exist |
| `AUDIO_SETTINGS_UPDATE` | Per-user volume and local mute synced from other clients |
| `USER_SETTINGS_UPDATE` | Not needed while the `USER_SETTINGS_PROTO` capability is set; listed so it isn't wired up by mistake |

## Inbox

| Event | Will be used for |
|---|---|
| `NOTIFICATION_CENTER_ITEM_CREATE` / `_DELETE` | The inbox's For You tab |
| `NOTIFICATION_CENTER_ITEMS_ACK`, `NOTIFICATION_CENTER_ITEM_COMPLETED` | Marking For You items read or done when it happens on another client |
| `USER_NON_CHANNEL_ACK` | The inbox's unread dot, and other read states that aren't a channel |

## Account, billing and apps

These only matter once the matching settings pages exist.

| Event | Will be used for |
|---|---|
| `AUTHENTICATOR_CREATE` / `_UPDATE` / `_DELETE` | Security keys and passkeys in My Account |
| `OAUTH2_TOKEN_CREATE` / `_DELETE` / `_REVOKE` | Authorized apps settings page |
| `USER_APPLICATION_IDENTITY_UPDATE` / `_REMOVE` | Per-app identities shown on profiles |
| `USER_SUBSCRIPTIONS_UPDATE`, `USER_PAYMENT_SOURCES_UPDATE`, `PAYMENT_UPDATE` | Nitro and billing settings |
| `USER_PREMIUM_GUILD_SUBSCRIPTION_SLOT_*` | Boost slots in the Nitro settings and when boosting a server |
| `ENTITLEMENT_CREATE` / `_UPDATE` / `_DELETE` | Owned items: Nitro perks, shop items, app purchases |
| `GIFT_CODE_CREATE` / `_UPDATE` | Gift links in chat showing as claimed |
| `VIRTUAL_CURRENCY_BALANCE_UPDATE` | Orbs balance |
| `QUESTS_USER_STATUS_UPDATE` / `QUESTS_USER_COMPLETION_UPDATE` | Quests progress |

## Out of scope

Received by user accounts but with nothing in the app to drive: `LOBBY_*` (Social SDK lobbies),
`GAME_RELATIONSHIP_*`, `GAME_SERVER_*`, `EXPERIMENT_SESSION_OVERRIDE_*`, `CONSOLE_COMMAND_UPDATE`,
`CREATOR_MONETIZATION_RESTRICTIONS_UPDATE`, `DM_SETTINGS_UPSELL_SHOW`, `BILLING_POPUP_BRIDGE_CALLBACK`,
`USER_PAYMENT_BROWSER_CHECKOUT_DONE`, `USER_PAYMENT_CLIENT_ADD`, `USER_MERGE_OPERATION_COMPLETED`,
`INTERACTION_IFRAME_MODAL_CREATE`, `DELETED_ENTITY_IDS` (only useful with a local cache across
restarts), and `INITIAL_GUILD` (only sent when lazy guild loading is negotiated).
