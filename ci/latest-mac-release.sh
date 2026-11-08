#!/bin/sh
# The newest release that carries a macOS disk image with its update file (ci/sign-mac-update.sh), by its tag;
# nothing when none does. Every release made by CI carries both (ARCHITECTURE.md, "The macOS edition",
# "Distribution"); one without them (the Mac's job switched off, or a disk image attached without its update file)
# is passed over, so the download page and Sparkle's feed always offer a Mac version that installed apps can update
# to. Needs `gh` signed in to the repository.
set -eu
for tag in $(gh release list --limit 50 --json tagName,createdAt -q 'sort_by(.createdAt) | reverse | .[].tagName'); do
  assets=$(gh release view "$tag" --json assets -q '.assets[].name')
  if printf '%s\n' "$assets" | grep -q '\.dmg$' && printf '%s\n' "$assets" | grep -q '\.dmg\.update$'; then
    echo "$tag"
    exit 0
  fi
done
