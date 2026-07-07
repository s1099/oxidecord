## Oxidecord

WIP Cross platform native discord client built with rust gpui.
Contribitions are welcome.

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
- Sending images & fix memory leak with loading images
- Store token securely instead of a plaintext file right now
- Embeds
- DM's
- Videos
- Caching
- Themes https://longbridge.github.io/gpui-component/docs/theme#theme-registry
- Settings