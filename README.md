## Oxidecord

WIP Cross platform native discord client built with rust gpui.

## Running

1. Clone the repo: 
```bash
git clone https://github.com/s1099/oxidecord
cd oxidecord
```
2. Build and run
```bash
cargo run # debug build
# or
cargo run --release # release build
``` 
3. Binary can be found in `target/release` or `target/debug`

Voice calls link Opus, which is built from source, so the build also needs
CMake and a C compiler. If CMake is older than the installed Visual Studio it
won't know that generator so either update CMake, or build through Ninja with
`CMAKE_GENERATOR=Ninja`.

### TODO
- [x] Embeds
- [x] Dm's
- [x] Image rendering
- [x] Themes
- [x] Updater
- [ ] App icon
- [ ] Status changes
- [ ] Video playback
- [x] Voice calls
- [ ] Screenshare and video calls
- [ ] Caching
- [ ] Markdown rendering
- [ ] Custom themes
- [ ] Cross platform autoupdater (only Windows is works right now)

