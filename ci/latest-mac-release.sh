#!/bin/sh
# The newest release that carries a macOS disk image, by its tag; nothing when none does. While the Mac is released
# by hand (ARCHITECTURE.md, "The macOS edition", "Distribution"), the releases after it carry none, so the download
# page offers this one's disk image, and pruning keeps it. Needs `gh` signed in to the repository.
set -eu
for tag in $(gh release list --limit 50 --json tagName,createdAt -q 'sort_by(.createdAt) | reverse | .[].tagName'); do
  if gh release view "$tag" --json assets -q '.assets[].name' | grep -q '\.dmg$'; then
    echo "$tag"
    exit 0
  fi
done
