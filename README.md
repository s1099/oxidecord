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

### TODO
- App icon
- Settings page
- Embeds
- Videos
- Voice calls
- Caching
- Themes https://longbridge.github.io/gpui-component/docs/theme#theme-registry
- Cross platform autoupdater (only Windows is implemented; macOS and Linux still need their own swap step)