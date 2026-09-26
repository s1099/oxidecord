# Missing Features

## Messaging

| Feature | Status | Notes |
|---|---|---|
| Syntax highlighting in code blocks | Missing | Code blocks and inline code render as plain monospace; the language tag is parsed but unused |
| Spoiler attachments | Missing | Spoiler text is done |
| Mention-highlighted messages (the tinted row when you're mentioned) | Missing | Mentions, `@everyone` and `@here` render as pills, but the row isn't highlighted |
| Local time zone for timestamps | Missing | `<t:…>` markup, like every other time in the app, is shown in UTC |
| Global display names in mentions | Partial | twilight's `Mention` model drops `global_name`, so a mention shows the nickname or username |
| Emoji picker | Missing | No way to insert emoji in the composer |
| Adding a new reaction to a message | Partial | Existing reactions can be toggled; there's no picker to add another |
| Viewing who reacted | Missing | |
| Super reactions (burst) | Missing | |
| Autocomplete for `@mentions`, `#channels`, `:emoji:` in the composer | Missing | |
| Slash commands and app commands | Missing | |
| Message components (buttons, select menus, modals) | Missing | Bot messages only show their embeds |
| Stickers (sending and rendering) | Missing | |
| GIF picker (Tenor) | Missing | |
| Polls (creating, voting, results) | Missing | |
| Forwarded messages | Missing | |
| Non-media file attachments shown in the chat (audio player, file cards for PDFs, zips, and so on) | Missing | The message model only has images and videos; other attachments are dropped from the view |
| Voice messages (recording and playback) | Missing | |
| Uploads larger than 10 MB (Nitro and boosted-server limits) | Partial | Hard-coded 10 MB cap (`MAX_ATTACHMENT_SIZE`) |
| Drag and drop of files onto the chat | Missing | Picker and paste only |
| Attachment alt text and descriptions | Missing | |
| Image lightbox and full-size viewer | Missing | |
| System messages (joins, pins, boosts, calls, thread created, and so on) | Missing | |
| Live message deletion (`MESSAGE_DELETE`, `MESSAGE_DELETE_BULK`) | Missing | Gateway ignores them; deleted messages stay until reload |
| Live reaction updates (`MESSAGE_REACTION_ADD/REMOVE`) | Missing | Other users' reactions don't appear until reload |
| Pinning and unpinning messages, and the pinned-messages panel | Missing | |
| Message search (with filters: from, in, has, before, after) | Missing | |
| Jump to a message (from a reply, a link, or search) | Missing | Clicking a reply quote doesn't scroll to the original |
| "Jump to present" and the new-messages bar | Missing | |
| Mark as unread | Missing | |
| Copy text, copy message ID, and other developer-mode context-menu items | Missing | Only "Copy Message Link" is there |
| Right-click context menus (message, user, channel, server) | Missing | Actions are on the hover toolbar and "More" menu only |
| Report message | Missing | |
| Suppress embeds on your own message | Missing | |
| Scheduled and silent messages (`@silent`) | Missing | |
| Send failure retry and a local pending-message state | Missing | |
| Slowmode indicator and cooldown | Missing | |
| Typing indicator ("X is typing…") and sending typing events | Missing | `TYPING_START` is dropped in the gateway |
| Message requests and filtering DMs from strangers | Missing | |

## Channels and servers

| Feature | Status | Notes |
|---|---|---|
| Threads (browsing, creating, joining, thread sidebar) | Missing | Thread channel types are filtered out in `convert_channel` |
| Forum channels (post list, tags, creating posts) | Partial | Listed with an icon, but selecting one tries to load messages like a text channel |
| Media channels | Missing | |
| Stage channels (speakers, audience, raising a hand) | Partial | Joined as a plain voice channel |
| Announcement channel follow and publish | Missing | Listed and readable only |
| Unread indicators (channel bold, server pill, unread divider) | Missing | No read-state tracking at all |
| Mention badges on servers, channels and DMs | Missing | |
| Server and channel notification settings (all, mentions, nothing, suppress `@everyone`) | Missing | |
| Muting servers, channels and DMs | Missing | |
| Member list sidebar with roles, presence and grouping | Missing | |
| Role colors on usernames | Missing | |
| Role icons, server tags and badges next to names | Missing | |
| Server banner and header dropdown menu | Missing | |
| Server boosting and boost level perks | Missing | |
| Joining servers via invite link, and creating invites | Missing | |
| Creating, leaving and deleting servers | Missing | |
| Server discovery | Missing | |
| Server settings (overview, roles, emoji, stickers, moderation, audit log, integrations, webhooks) | Missing | |
| Channel creation, editing, permissions overrides, reordering | Missing | |
| Moderation actions (kick, ban, timeout, AutoMod) | Missing | |
| Server onboarding, rules screening and channel browser | Missing | |
| Scheduled events | Missing | |
| Server folders: creating, editing, renaming, recoloring | Partial | Folders from settings are shown; they can't be edited |
| Dragging to reorder servers and channels | Missing | |
| Live guild and channel updates (`GUILD_CREATE/DELETE`, `CHANNEL_CREATE/UPDATE/DELETE`, role changes) | Missing | Only voice states are read from `GUILD_CREATE` |
| NSFW / age-gated channel gate | Missing | |

## Direct messages and social

| Feature | Status | Notes |
|---|---|---|
| Friends list (online, all, pending, blocked) | Missing | |
| Sending, accepting and removing friend requests | Missing | |
| Blocking and ignoring users | Missing | |
| Starting a new DM or group DM | Missing | Only existing DMs are listed |
| Group DM management (add or remove members, rename, change icon, leave) | Missing | |
| Closing (hiding) a DM from the list | Missing | |
| Active Now panel | Missing | |
| Live updates to the DM list (new DM, reordering by recent activity) | Missing | The list is fetched once per session |
| Notes on users | Missing | |
| Mutual servers and mutual friends in profiles | Missing | |
| Full user profile modal (activity, connections, member-since, roles) | Partial | Popout has a banner, bio, pronouns and creation date |
| Server-specific profile (nickname, server avatar, roles) in the popout | Missing | |
| Avatar decorations, profile effects, nameplates | Missing | |

## Presence and status

| Feature | Status | Notes |
|---|---|---|
| Setting your own status (online, idle, do not disturb, invisible) | Missing | Listed on the README TODO |
| Custom status | Missing | |
| Showing other users' presence (status dots) | Missing | `PRESENCE_UPDATE` is dropped |
| Rich presence and activities (games, Spotify, streaming) | Missing | |
| Detecting and broadcasting your own game activity | Missing | |
| Idle detection (auto-idle when AFK) | Missing | |

## Voice and video

| Feature | Status | Notes |
|---|---|---|
| Video calls (camera) | Missing | Button present but disabled |
| Screen sharing (Go Live) and watching streams | Missing | Button present but disabled |
| Output device selection | Missing | Only an input device picker |
| Input and output volume sliders | Missing | |
| Per-user volume and local mute | Missing | |
| Push to talk and keybinds | Missing | |
| Voice activity sensitivity threshold | Missing | |
| Noise suppression (Krisp), echo cancellation, automatic gain control | Missing | |
| Mic test and input level meter | Missing | |
| Server mute and deafen, moving and disconnecting members (moderator actions) | Missing | Server mute/deaf state is read but can't be set |
| Voice channel user limit and bitrate display | Missing | |
| Soundboard | Missing | |
| Activities (embedded apps in voice) | Missing | |
| Incoming call ringing and accept or decline UI | Missing | |
| Join and leave sound effects | Missing | |
| Voice channel text chat | Missing | |
| Picture-in-picture and pop-out call window | Missing | |
| Voice region override | Missing | |

## Notifications

| Feature | Status | Notes |
|---|---|---|
| Desktop notifications for mentions and DMs | Missing | |
| Notification sounds | Missing | |
| Taskbar and dock badge counts, flashing the window | Missing | |
| Inbox (mentions tab, unreads tab, For You) | Missing | |
| Do-not-disturb suppression | Missing | |

## App, settings and platform

| Feature | Status | Notes |
|---|---|---|
| Logging out and switching accounts | Missing | |
| Two-factor login (TOTP, SMS, passkey) on the token path | Partial | The webview login handles this through Discord's own page |
| QR code login | Missing | Webview may cover it |
| My Account settings (username, email, password, 2FA) | Missing | |
| Profile editing (avatar, banner, bio, pronouns, display name) | Missing | |
| Privacy and safety settings (DM filters, message requests) | Missing | |
| Authorized apps, sessions and devices | Missing | |
| Connections (Spotify, Steam, GitHub, and so on) | Missing | |
| Nitro, Shop and billing | Missing | |
| Custom themes and theme editor | Missing | Only built-in presets; listed on the README TODO |
| Appearance settings (compact mode, font scaling, message spacing, zoom level) | Missing | |
| Accessibility (reduced motion, saturation, role colors, text-to-speech, screen-reader support) | Missing | |
| Chat settings (display images, embeds, link previews, emoji autoconvert, sticker suggestions) | Missing | |
| Keybinds settings and global shortcuts (Ctrl+K quick switcher, Alt+Up/Down channel nav, Esc mark-as-read) | Missing | Only paste, send, save edit and edit-last-message are bound |
| Quick Switcher (Ctrl+K) | Missing | |
| Language and localization | Missing | |
| Streamer mode | Missing | |
| Developer mode (copy IDs) | Missing | |
| Local caching of guilds, channels and messages across restarts | Missing | Listed on the README TODO |
| Offline and reconnecting indicator | Missing | The shard reconnects silently |
| Minimize to system tray and run on startup | Missing | |
| Auto-updater on macOS and Linux | Missing | Windows only |
| Video playback on macOS and Linux | Missing | `platform/video/unsupported.rs` falls back to the poster card |
| App icon | Missing | Listed on the README TODO |
| Deep links (`discord://` URLs) and in-app handling of discord.com message links | Missing | |
| Multiple windows and popping out channels | Missing | |
| Spell check | Missing | |
| Clips and game overlay | Missing | |
| Family Center, Quests, Discovery, Nitro gifting | Missing | |
