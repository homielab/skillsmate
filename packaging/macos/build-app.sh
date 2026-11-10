#!/usr/bin/env bash
# Builds dist/SkillsMate.app from an existing release binary.
#
# Usage: packaging/macos/build-app.sh [path/to/skillsmate]
#
# The bundle is not signed:
# packaging/macos/build-app-store.sh signs it for the Mac App Store.
# BUILD_NUMBER sets CFBundleVersion (default: the version).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$ROOT/target/release/skillsmate}"
ICON="$ROOT/packaging/macos/AppIcon"
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

# App icon, compiled from packaging/macos/AppIcon.icon (see that folder's
# README): Assets.car for macOS 26+, AppIcon.icns for older versions.
cp "$ICON/Assets.car" "$ICON/AppIcon.icns" "$APP/Contents/Resources/"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>SkillsMate</string>
    <key>CFBundleDisplayName</key><string>SkillsMate</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID:-com.homielab.SkillsMate}</string>
    <key>CFBundleExecutable</key><string>skillsmate</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIconName</key><string>AppIcon</string>
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
