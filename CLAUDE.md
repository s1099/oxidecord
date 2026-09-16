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
- `src/platform/` — runtime, http client, prefs, updater.
- `src/ui/` — theming, settings, dialogs, shared widgets.
- `src/voice/` — songbird/cpal audio engine.
- `themes/` — JSON presets baked into the binary by `build.rs`.

## Threading

gpui's foreground thread is `!Send` and twilight/reqwest need Tokio, so a shared background
runtime lives in `platform/runtime.rs`. Every REST call spawns onto it and reports back
through a callback that runs **on that runtime's thread** — loaders in `data/` bridge it to
gpui over a `Send`-safe channel drained by a foreground task. Never block the foreground
thread, and never assume a callback runs on it.

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
