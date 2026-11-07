#!/bin/sh
# Sign a Mac disk image for Sparkle's updates (ARCHITECTURE.md, "The macOS edition", "Distribution"): an Ed25519
# signature over the whole file, which the app checks against its SUPublicEDKey, and the app's build number, which
# Sparkle compares with the installed one's. Both go beside the disk image as DMG.update, which is attached to the
# release with it; the download page's appcast is made from them.
#
#   ci/sign-mac-update.sh apple/build/release/immich-mount-2.15.0.dmg KEY.pem [BUILD]
#
# BUILD is the build number the disk image's app carries (CFBundleVersion, its commit's count, which
# apple/build-rust.sh writes into apple/build/Version.xcconfig); the release job passes the one it built with. Without
# it, the count at the release's tag is used, which must then be fetched: a wrong build number would make Sparkle
# offer the update wrongly, or never.
set -eu
[ $# -eq 2 ] || [ $# -eq 3 ] || { echo "usage: ci/sign-mac-update.sh DMG KEY.pem [BUILD]" >&2; exit 2; }
dmg=$1
key=$2
version=$(basename "$dmg" .dmg | sed 's/^immich-mount-//')
if [ $# -eq 3 ]; then
  build=$3
else
  build=$(git rev-list --count "v$version") || { echo "no tag v$version here: fetch it, or give BUILD" >&2; exit 1; }
fi
signature=$(openssl pkeyutl -sign -inkey "$key" -rawin -in "$dmg" | base64 | tr -d '\n')
printf 'build=%s\nsignature=%s\n' "$build" "$signature" > "$dmg.update"
echo "wrote $dmg.update (version $version, build $build)"
