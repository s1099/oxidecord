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
  registered COM activator. `src/platform/notify/windows.rs:103`
- **Threads are treated as mention-only** — per-thread notification settings aren't read,
  so a message in a thread notifies only when it pings the user.
  `src/screens/home/data/notifications.rs:110`
- **System messages never notify** — Discord notifies of joins, boosts and similar in
  all-messages channels. `src/screens/home/data/notifications.rs:84`
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
