#!/usr/bin/env bash
# Builds dist/SkillsMate-<version>-x86_64.AppImage from an existing release binary.
#
# Usage: packaging/linux/build-appimage.sh [path/to/skillsmate]
#
# Requires: curl. Downloads linuxdeploy (pinned, checksum-verified) into
# .appimage-tools/ on first run. For broad compatibility, build the binary on
# the oldest distro you support (CI uses Ubuntu 22.04): an AppImage only runs
# on systems whose glibc is at least as new as the build machine's.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$ROOT/target/release/skillsmate}"
TOOLS="$ROOT/.appimage-tools"
APPDIR="$ROOT/target/AppDir"
DIST="$ROOT/dist"

# linuxdeploy publishes a rolling "continuous" release. To pin a build, set
# LINUXDEPLOY_VERSION to a release tag and LINUXDEPLOY_SHA256 to its checksum.
LINUXDEPLOY_VERSION="${LINUXDEPLOY_VERSION:-continuous}"
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/${LINUXDEPLOY_VERSION}/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_SHA256="${LINUXDEPLOY_SHA256:-}"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found. Run: cargo build --release -p skills-app" >&2
    exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"

mkdir -p "$TOOLS" "$DIST"
LINUXDEPLOY="$TOOLS/linuxdeploy-${LINUXDEPLOY_VERSION}-x86_64.AppImage"
if [[ ! -x "$LINUXDEPLOY" ]]; then
    echo "Downloading linuxdeploy ${LINUXDEPLOY_VERSION}..."
    curl -fsSL --retry 3 -o "$LINUXDEPLOY.part" "$LINUXDEPLOY_URL"
    if [[ -n "$LINUXDEPLOY_SHA256" ]]; then
        echo "${LINUXDEPLOY_SHA256}  $LINUXDEPLOY.part" | sha256sum -c -
    else
        echo "warning: LINUXDEPLOY_SHA256 not set; skipping checksum verification" >&2
    fi
    chmod +x "$LINUXDEPLOY.part"
    mv "$LINUXDEPLOY.part" "$LINUXDEPLOY"
fi

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"
cp "$ROOT/packaging/icons/skillsmate.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
for size in 16 32 48 64 128 256 512; do
    dir="$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    cp "$ROOT/packaging/icons/skillsmate-${size}.png" "$dir/skillsmate.png"
done

# APPIMAGE_EXTRACT_AND_RUN lets linuxdeploy run without FUSE (containers, CI).
# Windowing/GL libraries are deliberately not bundled: they must come from the
# host so they match its display server and GPU drivers.
export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH=x86_64
export LINUXDEPLOY_OUTPUT_VERSION="$VERSION"
(
    cd "$DIST"
    "$LINUXDEPLOY" \
        --appdir "$APPDIR" \
        --executable "$BIN" \
        --desktop-file "$ROOT/packaging/linux/skillsmate.desktop" \
        --icon-file "$ROOT/packaging/icons/skillsmate-256.png" \
        --icon-filename skillsmate \
        --output appimage
)

OUT="$(ls -t "$DIST"/SkillsMate-*.AppImage | head -n1)"
echo "Built $OUT"
