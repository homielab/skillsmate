#!/usr/bin/env bash
# Builds dist/SkillsMate.app from an existing release binary.
#
# Usage: packaging/macos/build-app.sh [path/to/skillsmate]
#
# Requires the Xcode command line tools (iconutil). The bundle is not signed:
# packaging/macos/build-app-store.sh signs it for the Mac App Store.
# BUILD_NUMBER sets CFBundleVersion (default: the version).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$ROOT/target/release/skillsmate}"
ICONS="$ROOT/packaging/icons"
DIST="$ROOT/dist"
APP="$DIST/SkillsMate.app"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found. Run: cargo build --release -p skills-app" >&2
    exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"

rm -r "$APP" 2>/dev/null || true
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/skillsmate"

# App icon from the rendered PNGs.
ICONSET="$(mktemp -d)/SkillsMate.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    cp "$ICONS/skillsmate-$size.png" "$ICONSET/icon_${size}x${size}.png"
done
cp "$ICONS/skillsmate-32.png" "$ICONSET/icon_16x16@2x.png"
cp "$ICONS/skillsmate-64.png" "$ICONSET/icon_32x32@2x.png"
cp "$ICONS/skillsmate-256.png" "$ICONSET/icon_128x128@2x.png"
cp "$ICONS/skillsmate-512.png" "$ICONSET/icon_256x256@2x.png"
cp "$ICONS/skillsmate-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/SkillsMate.icns"
rm -r "$(dirname "$ICONSET")"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>SkillsMate</string>
    <key>CFBundleDisplayName</key><string>SkillsMate</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID:-com.homielab.SkillsMate}</string>
    <key>CFBundleExecutable</key><string>skillsmate</string>
    <key>CFBundleIconFile</key><string>SkillsMate</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleVersion</key><string>${BUILD_NUMBER:-$VERSION}</string>
    <key>NSHumanReadableCopyright</key><string>Copyright © 2026 HomieLab.com</string>
    <key>ITSAppUsesNonExemptEncryption</key><false/>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
EOF

echo "Built $APP"
