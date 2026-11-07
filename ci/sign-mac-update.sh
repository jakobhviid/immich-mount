#!/bin/sh
# Sign a Mac disk image for Sparkle's updates (ARCHITECTURE.md, "The macOS edition", "Distribution"): an Ed25519
# signature over the whole file, which the app checks against its SUPublicEDKey, and the app's build number, which
# Sparkle compares with the installed one's. Both go beside the disk image as DMG.update, which is attached to the
# release with it; the download page's appcast is made from them.
#
#   ci/sign-mac-update.sh apple/build/release/immich-mount-2.15.0.dmg KEY.pem
set -eu
[ $# -eq 2 ] || { echo "usage: ci/sign-mac-update.sh DMG KEY.pem" >&2; exit 2; }
dmg=$1
key=$2
version=$(basename "$dmg" .dmg | sed 's/^immich-mount-//')
# The build number the disk image's app carries: its commit's count (apple/build-rust.sh), its release's tag once there.
build=$(git rev-list --count "v$version" 2>/dev/null || git rev-list --count HEAD)
signature=$(openssl pkeyutl -sign -inkey "$key" -rawin -in "$dmg" | base64 | tr -d '\n')
printf 'build=%s\nsignature=%s\n' "$build" "$signature" > "$dmg.update"
echo "wrote $dmg.update (version $version, build $build)"
