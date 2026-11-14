#!/usr/bin/env bash
# Builds target/app-store/SkillsMate-<version>-<build>.pkg for the Mac App
# Store: a universal, sandboxed app signed by Xcode's automatic signing with an
# App Store Connect API key (no local certificates needed).
#
# Usage: BUILD_NUMBER=<n> packaging/macos/build-app-store.sh
#
# Environment:
#   ASC_KEY_ID, ASC_ISSUER_ID  App Store Connect API key (Admin or App Manager)
#   ASC_KEY_PATH               default ~/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8
#   BUILD_NUMBER               CFBundleVersion; must grow with every upload
#   TEAM_ID                    default 79LQ4MHVMG (HomieLab)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEAM_ID="${TEAM_ID:-79LQ4MHVMG}"
BUNDLE_ID="com.homielab.SkillsMate"
: "${ASC_KEY_ID:?Set ASC_KEY_ID}" "${ASC_ISSUER_ID:?Set ASC_ISSUER_ID}" "${BUILD_NUMBER:?Set BUILD_NUMBER}"
ASC_KEY_PATH="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
WORK="$ROOT/target/app-store"

cd "$ROOT"
rm -r "$WORK" 2>/dev/null || true
mkdir -p "$WORK"

# Universal, unsigned .app with the sandbox feature and this build number.
bun install --frozen-lockfile
bunx tauri build --target universal-apple-darwin --bundles app --no-sign \
    --features app-store \
    --config "{\"bundle\":{\"macOS\":{\"bundleVersion\":\"${BUILD_NUMBER}\"}}}"

# Sign ad hoc with the sandbox entitlements: the export step keeps them when it
# re-signs for distribution.
ARCHIVE="$WORK/SkillsMate.xcarchive"
mkdir -p "$ARCHIVE/Products/Applications"
mv "$ROOT/target/universal-apple-darwin/release/bundle/macos/SkillsMate.app" \
    "$ARCHIVE/Products/Applications/"
APP="$ARCHIVE/Products/Applications/SkillsMate.app"
codesign --force --sign - --options runtime \
    --entitlements packaging/macos/AppStore.entitlements "$APP"

cat > "$ARCHIVE/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>ApplicationProperties</key>
    <dict>
        <key>ApplicationPath</key><string>Applications/SkillsMate.app</string>
        <key>Architectures</key><array><string>arm64</string><string>x86_64</string></array>
        <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
        <key>CFBundleShortVersionString</key><string>${VERSION}</string>
        <key>CFBundleVersion</key><string>${BUILD_NUMBER}</string>
        <key>Team</key><string>${TEAM_ID}</string>
    </dict>
    <key>ArchiveVersion</key><integer>2</integer>
    <key>CreationDate</key><date>$(date -u +%Y-%m-%dT%H:%M:%SZ)</date>
    <key>Name</key><string>SkillsMate</string>
    <key>SchemeName</key><string>SkillsMate</string>
</dict>
</plist>
PLIST

cat > "$WORK/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>method</key><string>app-store-connect</string>
    <key>destination</key><string>export</string>
    <key>teamID</key><string>${TEAM_ID}</string>
    <key>signingStyle</key><string>automatic</string>
</dict>
</plist>
PLIST

xcodebuild -exportArchive \
    -archivePath "$ARCHIVE" \
    -exportPath "$WORK/export" \
    -exportOptionsPlist "$WORK/ExportOptions.plist" \
    -allowProvisioningUpdates \
    -authenticationKeyPath "$ASC_KEY_PATH" \
    -authenticationKeyID "$ASC_KEY_ID" \
    -authenticationKeyIssuerID "$ASC_ISSUER_ID"

PKG="$WORK/SkillsMate-${VERSION}-${BUILD_NUMBER}.pkg"
mv "$WORK/export/SkillsMate.pkg" "$PKG"
echo "Built $PKG"
