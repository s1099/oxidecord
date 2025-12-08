## Oxidecord

WIP Cross platform native discord client built with gpui rust that aims to be blazing fast and have low memory footprint.

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
cargo run --release
``` 
3. If needed, you can find the binary in `target/release` or `target/debug`

### TODO
- Fix message height calc
- Image rendering
- Render guild logos
- Proper login screen, webview login
- User login support (only works with bot accounts right now)
- DM's
- Caching
- Themes https://longbridge.github.io/gpui-component/docs/theme#theme-registry
- Settings