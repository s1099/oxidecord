#!/bin/sh
# Builds target/macos-bundle/release/Oxidecord.app. A bare binary gets the generic
# executable icon, so the Dock icon only appears when launched from a bundle.
#
# assets/AppIcon.icon is an Icon Composer document; actool compiles it to
# Assets.car (the macOS 26 glass icon) plus AppIcon.icns for older systems.
# Needs Xcode 26 or later for that.
set -eu

cd "$(dirname "$0")/.."

# Built paths end up in the binary, which would put the home directory (and so
# the username) in an app meant to be handed out:
# - Rust panic locations carry each crate's source path.
# - Opus is C, built by CMake, and its asserts carry __FILE__.
# - gpui's precompiled Metal shaders carry debug line tables;
#   `runtime_shaders` ships the source and compiles it at launch instead.
# A target dir of its own, since these flags would otherwise invalidate every
# ordinary build, and Opus's build script doesn't rerun when CFLAGS changes.
target=target/macos-bundle
RUSTFLAGS="--remap-path-prefix=$HOME=~ ${RUSTFLAGS:-}" \
    CFLAGS="-ffile-prefix-map=$HOME=~ ${CFLAGS:-}" \
    cargo build --release --target-dir "$target" --features gpui/runtime_shaders

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=$target/release/Oxidecord.app

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/oxidecord" "$app/Contents/MacOS/oxidecord"

xcrun actool assets/AppIcon.icon \
    --compile "$app/Contents/Resources" \
    --app-icon AppIcon \
    --platform macosx \
    --target-device mac \
    --minimum-deployment-target 11.0 \
    --output-partial-info-plist /dev/null \
    > /dev/null

# A bundle without a microphone usage string is killed by macOS the moment
# voice opens the input device.
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Oxidecord</string>
    <key>CFBundleDisplayName</key>
    <string>Oxidecord</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.s1099.oxidecord</string>
    <key>CFBundleExecutable</key>
    <string>oxidecord</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIconName</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Oxidecord uses the microphone for voice calls.</string>
</dict>
</plist>
EOF

# Ad-hoc, so the bundle's contents match its signature and Gatekeeper doesn't
# report it as damaged. Not a Developer ID signature: it runs here, but
# another Mac will ask to approve it in Privacy & Security.
codesign --force --sign - "$app"

echo "Built $app"
