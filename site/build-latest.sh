#!/bin/sh
# Build the download page for the latest release into OUTPUT (site/build.py), with the Mac download and the update
# appcast from the newest release that has a disk image (ci/latest-mac-release.sh). Needs `gh` signed in.
#
#   site/build-latest.sh OUTPUT
set -eu
output=$1
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release view --json tagName,publishedAt,url,assets > "$work/release.json"
mac=$(ci/latest-mac-release.sh)
if [ -z "$mac" ]; then
  python3 site/build.py "$work/release.json" "$output"
  exit 0
fi
gh release view "$mac" --json tagName,publishedAt,url,assets > "$work/macos.json"
if gh release download "$mac" --pattern '*.dmg.update' --dir "$work" 2>/dev/null; then
  python3 site/build.py "$work/release.json" "$output" --macos "$work/macos.json" --update "$work"/*.dmg.update
else
  python3 site/build.py "$work/release.json" "$output" --macos "$work/macos.json"
fi
