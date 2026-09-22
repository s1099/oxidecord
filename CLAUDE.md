# Oxidecord

A cross-platform native Discord client in Rust, built on [gpui](https://crates.io/crates/gpui)
and [gpui-component](https://crates.io/crates/gpui-component), talking to Discord through
[twilight](https://github.com/twilight-rs/twilight) (patched to a fork that allows user tokens).

## Commands

```bash
cargo run              # debug build, keeps a console for logging
cargo run --release    # release build, no console
cargo check            # fast type check — prefer this while iterating
cargo fmt
cargo clippy
```

Voice links Opus, built from source, so the build needs CMake and a C compiler. If CMake is
older than the installed Visual Studio, build through Ninja: `CMAKE_GENERATOR=Ninja`.

## Layout

- `src/discord/` — the Discord client: `model/` (app-side types), `rest/` (HTTP calls),
  `gateway.rs` (live events), `token.rs` (keyring storage). The app's own types are defined
  here rather than using twilight's directly.
- `src/screens/home/` — the main screen. `state.rs` owns all state; `data/` mutates it and
  talks to `discord`; `view/` renders it. Both extend `HomeScreen` with inherent methods.
- `src/platform/` — runtime, http client, prefs, updater, and `video/` (inline video
  playback, decoded by the OS).
- `src/ui/` — theming, settings, dialogs, shared widgets.
- `src/voice/` — songbird/cpal audio engine.
- `themes/` — JSON presets baked into the binary by `build.rs`.

## Threading

gpui's foreground thread is `!Send` and twilight/reqwest need Tokio, so a shared background
runtime lives in `platform/runtime.rs`. `runtime::run` spawns a future onto it and hands
back one gpui can await, so the REST calls in `discord/rest/` are plain `async fn`s that
loaders in `data/` await inside `cx.spawn`. Never block the foreground thread.

## Video playback

`platform/video/` plays video attachments inline. Nothing here ships a codec — Windows
decodes through Media Foundation (`windows.rs`), so hardware acceleration comes free and
there's no redistributable or licensing to carry. `unsupported.rs` is the fallback for other
platforms (AVFoundation and GStreamer would fit the same shape); it returns an error that
sends the UI back to the poster card and its "open externally" action.

- One `VideoPlayer` drives one clip on its own `video-decode` thread: it downloads the
  attachment to a temp file first (a seekable local file makes scrubbing work without
  byte-ranges), opens the speakers, decodes, and reports back over a channel as
  `VideoEvent`s. Dropping it stops the thread, closes the device, and deletes the file.
- Control state is atomics in `Control`, not a lock — the decoder and the audio callback
  both read it, and neither can wait on the UI thread.
- Frames arrive as tightly packed BGRA already scaled to the size they'll be drawn at, which
  is the whole cost argument: a few hundred kilobytes per frame instead of megabytes. Media
  Foundation leaves `RGB32`'s fourth byte at zero, which gpui draws as fully transparent, so
  the backend forces alpha opaque.
- Playback is paced against the output device's clock (`clock.rs`), counting frames actually
  handed to the speakers, so pausing and starving cost nothing. A file with no audio track
  falls back to a wall clock. `output.rs` is a separate cpal stream from `voice/`, with
  shallower buffering.
- Only one clip plays at a time (`HomeScreen::play_video` stops the previous one), and at
  most `MAX_FRAMES_IN_FLIGHT` frames are outstanding to the UI. Every frame occupies a gpui
  sprite atlas slot, so the view must hand the old one back via `window.drop_image` and call
  `frame_consumed` — miss that and the decoder stalls and the atlas leaks.

## Conventions

- Comments explain *why*, not what — non-obvious constraints, workarounds, and the reasoning
  behind a choice. Module-level `//!` docs describe what a module owns. Match that density;
  don't narrate obvious code.
- Home screen state is `pub(super)`; keep it that way.
- Commits: conventional lowercase (`feat:`, `fix:`, `chore:`).
- Run `cargo fmt` and `cargo clippy` before committing.

## UI work

Use the `gpui-component` skill (`.claude/skills/`) when building UI — component APIs and the
style guide live there. Prefer existing gpui-component widgets over hand-rolled elements, and
theme colors (`cx.theme()`) over literals.
