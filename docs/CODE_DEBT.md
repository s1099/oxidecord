# Code debt

Gaps left in features that already shipped: shortcuts, unhandled edge cases, known bugs,
hacks and TODOs. Entries are grouped by feature. Add one when work leaves a gap behind, and
delete it once the gap is closed. Features the app doesn't have at all go in
[`MISSING_FEATURES.md`](MISSING_FEATURES.md).

Entry format:

```md
## <Feature name>

- **<Short gap title>** — what's missing or wrong, and why it was left. `src/path.rs:123`
```

## Chat scrollbar

- **Clicking the track jumps instead of paging** — gpui-kit's `Scrollbar` centers the thumb on
  the click, where Discord (a browser scrollbar) pages one screen toward it. Paging needs a
  scrollbar of our own. `src/screens/home/view/content/message_list.rs:166`

## Message components

- **Modal file uploads can't be filled** — the field shows a notice, and a required one blocks
  submitting. `src/screens/home/view/components/modal.rs:132`
- **Modal text displays are plain text** — the markdown renderer needs the home screen's
  context, which the modal's entity doesn't have. `src/screens/home/view/components/modal.rs:510`
- **Radio groups and iframe modals are dropped** — twilight has no radio group (type 21), so the
  field is left out of the form and of the submission; `INTERACTION_IFRAME_MODAL_CREATE` isn't
  handled at all. `src/discord/model/component.rs:607`
- **Premium buttons are inert** — there's no store purchase flow. Link buttons open without
  Discord's "Leaving Discord" confirmation. `src/screens/home/view/message/components.rs:187`
- **Components only draw in the message list** — forwards, the pins popout and search results
  leave them out. `src/discord/model/message.rs:634`
- **Auto-populated selects offer only what the client knows** — users come from member search
  and the open conversation, roles and channels from the open guild's cache; a channel select in
  a DM offers nothing. Media alt text and a file component's spoiler flag are ignored.
  `src/screens/home/data/components.rs:263`

## Slash commands

- **Permissions are simplified** — commands are filtered by `contexts`, `default_member_permissions`
  against guild-wide permissions, and channel overwrites that deny; per-role and per-user
  overwrites are left to Discord, which omits most of them from the index already.
  `src/discord/model/command.rs:242`
- **Age-restricted commands are always hidden** — channels' NSFW flag isn't tracked.
  `src/screens/home/data/commands.rs:120`
- **File options can't be filled** — a command with an attachment option is refused before
  it's sent. `src/screens/home/commands.rs:322`
- **A bot DM's index isn't refreshed** — `GUILD_APPLICATION_COMMAND_INDEX_UPDATE` drops only
  that guild's index, not the DMs of bots in it. `src/discord/gateway/dispatch.rs:479`
- **Replies without a nonce are matched loosely** — an app's reply that doesn't echo the nonce
  takes the place of the newest command still waiting on the same bot, which could be the wrong
  one if two run at once. `src/screens/home/data/commands.rs:673`
- **Ephemeral messages keep the full toolbar** — replying, reacting and pinning fail on them,
  since Discord only knows them in this session. `src/screens/home/view/message/toolbar.rs:31`
- **Plain-text options are ambiguous** — a later word of a value that starts an option's name
  opens the option list, and Enter then takes the option instead of sending.
  `src/screens/home/commands.rs:207`

## Stickers

- **Recently used stickers are local** — kept in `prefs.json`, not synced through Discord's
  frecency settings, so other clients' picks don't show and ours don't reach them.
  `src/screens/home/data/stickers.rs:211`
- **No sticker popout** — clicking a sticker in a message does nothing; Discord opens a card
  naming its pack or guild. Hovering names it. `src/screens/home/view/message/sticker.rs:48`
- **GIF stickers' still is unverified** — the picker asks the media proxy for a WebP of a GIF
  sticker, which works for APNG but wasn't tried on a GIF one. `src/discord/model/sticker.rs:54`
- **Unsupported Lottie falls back to the name** — rasterlottie rejects expressions, effects and
  text animators; every Discord pack sticker renders, but a partner guild's could not.
  `src/screens/home/view/message/sticker.rs:84`

## Threads

- **Full view only** — a thread replaces its channel as the conversation, as Discord's "Open in
  Full View" does; the split view beside the channel would need a second message list and
  composer. `src/screens/home/data/threads.rs:162`
- **Previews only for threads heard from** — the threads list and the card under a starter
  message show the newest message only for threads a sync or a live message brought one for;
  others show their age. `src/screens/home/view/threads.rs:354`
- **New threads always archive after three days** — Discord's duration picker isn't offered.
  `src/discord/rest/thread.rs:26`
- **No thread in the quick switcher** — Ctrl+K lists channels and DMs only.
  `src/screens/home/data/quick_switcher.rs:1`
- **The first message bypasses the composer** — the "New Thread" panel's message is sent as
  typed, without mention or emoji resolution, attachments, or a pending copy.
  `src/screens/home/data/threads.rs:562`

## Activities

- **Loaded top-level, not framed** — the activity's page is the webview's document, with a shim
  standing in for `window.parent`, instead of an iframe inside a Discord-origin page. An activity
  that checks `window.top` or its referrer may behave differently than in Discord's client.
  `src/screens/activity/shim.js:1`
- **Only participant events are dispatched** — `SUBSCRIBE` succeeds for every event, but only
  `ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE` is ever sent; voice state, speaking, layout mode and
  current-user updates never arrive. `src/screens/activity/mod.rs:343`
- **`SET_ACTIVITY` goes nowhere** — the activity is told its rich presence was set, but presence
  updates always send empty activities. `src/screens/activity/mod.rs:376`
- **Purchases, invites and sharing are refused** — SKUs and entitlements answer empty, and
  invite, share, image upload, purchase and quest commands error as unsupported.
  `src/screens/activity/mod.rs:433`
- **`GET_CHANNEL` omits voice states and messages** — both arrays are always empty.
  `src/screens/activity/mod.rs:397`
- **Permissions are guild-wide** — the activities button and `GET_CHANNEL_PERMISSIONS` ignore
  channel overwrites. `src/screens/home/data/activities.rs:67`
- **Running activities only show in the call's menu** — the channel list and stage don't mark a
  channel with an activity in it. `src/screens/home/view/voice/activities.rs:85`
- **External links open without asking** — `OPEN_EXTERNAL_LINK` and navigations off the
  activity's origin go straight to the browser, where Discord's client asks first.
  `src/screens/activity/mod.rs:379`

## Markdown

- **Shortcodes use gemoji's names** — `:name:` in received text resolves through the `emojis`
  crate plus Discord's `flag_xx`; names only Discord's own table has stay text.
  `src/discord/model/markdown/inline.rs:592`

## Message links

- **A closed DM opened from a link can drop out of the list** — it's added to `dms.list`, but if
  the DM list hadn't loaded yet, the load replaces the list and the open DM loses its header.
  `src/screens/home/data/messages/jump.rs:105`

## Message grouping

- **Day dividers lag a time zone change** — the local day last looked up is remembered by the
  seconds it spans, so changing the system time zone mid-session leaves dividers on the old
  zone's midnight until another day is asked about. `src/discord/model/time.rs:37`

## Close to tray

- **No setting to quit on close** — closing the main window always hides it to the tray;
  Discord offers a toggle. `src/platform/tray.rs:31`
- **Inline video keeps playing while hidden** — hiding doesn't pause the clip, so its audio
  goes on with no window to stop it from. `src/platform/tray.rs:32`

## Desktop notifications

- **Only on Windows** — other platforms have no backend, so their notifications are
  dropped. `src/platform/notify/unsupported.rs:9`
- **Clicks only work while the app runs** — a toast is opened through its `Activated`
  event, so one clicked in Action Center after a restart does nothing. Fixing that needs a
  registered COM activator. `src/platform/notify/windows.rs:128`
- **Threads are treated as mention-only** — per-thread notification settings aren't read,
  so a message in a thread notifies only when it pings the user.
  `src/screens/home/data/notifications.rs:110`
- **System messages never notify** — Discord notifies of joins, boosts and similar in
  all-messages channels. `src/screens/home/data/notifications.rs:84`
- **No trailing sound after a burst** — Discord's throttle plays one more sound at the end
  of a burst's second; here the sound rides on the toast, so the burst's later toasts are
  just silent. `src/platform/notify/windows.rs:67`
- **Avatar cache never shrinks** — senders' avatars pile up in the temp directory.
  `src/platform/notify/mod.rs:77`

## Copying images

- **Failures are silent** — a download or decode that fails (an AVIF from a third-party
  host, say) only logs to the console; a toast would sit under the viewer's overlay.
  `src/screens/home/data/image_viewer.rs:233`
- **Clicking off the menu also closes the viewer** — the press that dismisses the
  right-click menu reaches the scrim too. `src/screens/home/view/image_viewer.rs:92`
- **Animated images copy their first frame** — only PNG and a bitmap go on the clipboard,
  as Discord's own "Copy Image" does. `src/platform/clipboard.rs:89`
