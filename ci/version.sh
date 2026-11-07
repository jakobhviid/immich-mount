#!/bin/sh
# The version, from the commit subjects (AGENTS.md, "Versioning"): the first commit is 1.0.0, then a breaking
# `type!:` bumps the major version, `feat:` the minor, anything else the patch. Every release job and every Mac build
# take it from here, so the Linux packages and the macOS app carry one version. It needs the whole history: a
# shallow checkout counts from 1.0.0.
#
#   ci/version.sh            print the version, such as 2.1.0
#   ci/version.sh --stamp    also write it into Cargo.toml, so `immich-mount --version` says it
set -eu
cd "$(dirname "$0")/.."

version=$(git log --reverse --format='%H%x09%s' | awk -F'\t' '
  BEGIN { major = 1; minor = 0; patch = 0 }
  NR == 1 { next }
  {
    subject = $2
    if (subject ~ /^[A-Za-z]+(\([^)]*\))?!:/) { major++; minor = 0; patch = 0 }
    else if (subject ~ /^feat(\([^)]*\))?:/) { minor++; patch = 0 }
    else { patch++ }
  }
  END { print major "." minor "." patch }')

if [ "${1:-}" = --stamp ]; then
  stamped=$(mktemp)
  sed -E "s/^version = \"[^\"]*\"/version = \"$version\"/" Cargo.toml > "$stamped" && mv "$stamped" Cargo.toml
fi
echo "$version"
