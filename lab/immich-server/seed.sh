#!/usr/bin/env bash
# Seed a fresh server from ./up.sh: admin, two API keys, Commons fixtures, albums,
# an archived and a trashed asset, a manual person. Writes .env.local (URL + keys).
#
#   ./seed.sh
#
# Env overrides: IMMICH_INSTANCE (default ind-immich-srv), EREBUS_DIR,
#   WIKIMEDIA_UA  User-Agent for Commons downloads (their policy asks for a descriptive one)
#
# Fixtures: fixtures/manifest.json pins every file (URL, sha1, author, license, role).
# They are downloaded once into fixtures/cache/ (git-ignored), checked against the sha1,
# copied into the container, and uploaded from there, so NAT mode works too.
# Photos are Commons thumbnails; seed-container.sh writes the original's camera EXIF back.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
NAME=${IMMICH_INSTANCE:-ind-immich-srv}
UA=${WIKIMEDIA_UA:-"ImmichDesktopNautilus-lab-fixtures/0.1 (test fixtures for a development lab; low volume)"}
P=lab
# shellcheck source=/dev/null
. "$EREBUS_DIR/lib/erebus.sh"

[ "$(state "$P" "$NAME")" = RUNNING ] || die "$NAME is not running; run ./up.sh first"
in_ct() { inc exec --project "$P" "$NAME" -- "$@" </dev/null; }
IP=$(inc list --project "$P" "^$NAME\$" -f json </dev/null |
     jq -r '.[0].state.network.eth0.addresses[] | select(.family=="inet") | .address' | head -1)

step "fixtures: download missing files into fixtures/cache and verify sha1"
mkdir -p "$HERE/fixtures/cache"
jq -r '.files[] | [.url, .cacheName, .sha1, .optional] | @tsv' "$HERE/fixtures/manifest.json" |
while IFS=$'\t' read -r url file sha1 optional; do
  f="$HERE/fixtures/cache/$file"
  if [ -f "$f" ] && [ "$(sha1sum < "$f" | cut -d' ' -f1)" = "$sha1" ]; then continue; fi
  if [ "$optional" = true ]; then
    # Originals (the two WebM videos) come from upload.wikimedia.org, which answers bulk
    # clients with 429 + Retry-After: 600. One try; skip on failure, a later run fetches them.
    curl -fsSL -A "$UA" -o "$f.part" "$url" </dev/null ||
      { rm -f "$f.part"; echo "         skipped optional $file (Commons refused; retry later)"; continue; }
  else
    # thumb.wikimedia.org; --retry without --retry-delay honours Retry-After
    curl -fsSL --retry 3 -A "$UA" -o "$f.part" "$url" </dev/null
  fi
  got=$(sha1sum < "$f.part" | cut -d' ' -f1)
  [ "$got" = "$sha1" ] || die "sha1 mismatch for $url: $got (manifest $sha1)"
  mv "$f.part" "$f"; echo "         fetched $file"
  sleep 1   # be gentle with Commons
done

step "copy fixtures and seed-container.sh into $NAME"
in_ct mkdir -p /opt/fixtures
tar -C "$HERE/fixtures" -cf - manifest.json cache | inc exec --project "$P" "$NAME" -- tar -xf - -C /opt/fixtures
inc file push --project "$P" "$HERE/seed-container.sh" "$NAME/opt/immich/seed-container.sh" </dev/null

step "setup: admin, ML off, API keys"
in_ct bash /opt/immich/seed-container.sh setup
step "upload"
in_ct bash /opt/immich/seed-container.sh upload
wait_for "$P" "$NAME" "job queues idle (metadata, thumbnails, geocoding)" bash /opt/immich/seed-container.sh idle
step "organize: albums, archive, trash, person"
in_ct bash /opt/immich/seed-container.sh organize
wait_for "$P" "$NAME" "job queues idle" bash /opt/immich/seed-container.sh idle

step "write .env.local"
{
  echo "# Throwaway credentials for the disposable lab server $NAME. Written by seed.sh."
  echo "IMMICH_URL=http://$IP:2283"
  in_ct cat /opt/immich/seed/keys.env
} > "$HERE/.env.local"
chmod 600 "$HERE/.env.local"
in_ct cat /opt/immich/seed/assets.tsv > "$HERE/.assets.tsv"   # manifest id -> asset id, for tests
step "$(in_ct bash /opt/immich/seed-container.sh summary)"
