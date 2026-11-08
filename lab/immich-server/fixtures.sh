#!/usr/bin/env bash
# Download the seed's fixtures into fixtures/cache (git-ignored) and check each against its sha1 in
# fixtures/manifest.json. Used by seed.sh (the lab) and ci.sh (CI); a file already cached is not fetched again.
#
#   ./fixtures.sh
#
# Env: WIKIMEDIA_UA, the User-Agent for Commons downloads (their policy asks for a descriptive one).
#      FIXTURES_ALL=1 makes the optional files required too, waiting out Commons' Retry-After (CI: the contract
#      tests count both videos).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
UA=${WIKIMEDIA_UA:-"ImmichDesktopNautilus-lab-fixtures/0.1 (test fixtures for a development lab; low volume)"}

mkdir -p "$HERE/fixtures/cache"
jq -r '.files[] | [.url, .cacheName, .sha1, .optional] | @tsv' "$HERE/fixtures/manifest.json" |
while IFS=$'\t' read -r url file sha1 optional; do
  f="$HERE/fixtures/cache/$file"
  if [ -f "$f" ] && [ "$(sha1sum < "$f" | cut -d' ' -f1)" = "$sha1" ]; then continue; fi
  if [ "$optional" = true ] && [ "${FIXTURES_ALL:-}" = 1 ]; then
    # --retry honours Retry-After (up to ten minutes from upload.wikimedia.org); bounded overall.
    curl -fsSL --retry 4 --retry-max-time 1800 -A "$UA" -o "$f.part" "$url" </dev/null
  elif [ "$optional" = true ]; then
    # Originals (the two WebM videos) come from upload.wikimedia.org, which answers bulk
    # clients with 429 + Retry-After: 600. One try; skip on failure, a later run fetches them.
    curl -fsSL -A "$UA" -o "$f.part" "$url" </dev/null ||
      { rm -f "$f.part"; echo "         skipped optional $file (Commons refused; retry later)"; continue; }
  else
    # thumb.wikimedia.org; --retry without --retry-delay honours Retry-After
    curl -fsSL --retry 3 -A "$UA" -o "$f.part" "$url" </dev/null
  fi
  got=$(sha1sum < "$f.part" | cut -d' ' -f1)
  [ "$got" = "$sha1" ] || { echo "sha1 mismatch for $url: $got (manifest $sha1)" >&2; exit 1; }
  mv "$f.part" "$f"; echo "         fetched $file"
  sleep 1   # be gentle with Commons
done
