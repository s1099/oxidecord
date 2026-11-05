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

## Close to tray

- **No setting to quit on close** — closing the main window always hides it to the tray;
  Discord offers a toggle. `src/platform/tray.rs:31`
- **Inline video keeps playing while hidden** — hiding doesn't pause the clip, so its audio
  goes on with no window to stop it from. `src/platform/tray.rs:32`

## Copying images

- **Failures are silent** — a download or decode that fails (an AVIF from a third-party
  host, say) only logs to the console; a toast would sit under the viewer's overlay.
  `src/screens/home/data/image_viewer.rs:233`
- **Clicking off the menu also closes the viewer** — the press that dismisses the
  right-click menu reaches the scrim too. `src/screens/home/view/image_viewer.rs:92`
- **Animated images copy their first frame** — only PNG and a bitmap go on the clipboard,
  as Discord's own "Copy Image" does. `src/platform/clipboard.rs:89`
