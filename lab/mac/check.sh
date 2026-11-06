#!/usr/bin/env bash
# The macOS edition's check, from this machine: Finder's view of the lab server on the maintainer's Mac, one PASS or
# FAIL line per claim (the mac-dev skill). It needs, already in place: a signed build of the app, signed in to the lab
# server through the tunnels, running, and the extension switched on. It reads a few originals, and adds one photo to
# the lab server and deletes it again.
#
#   lab/mac/check.sh
#
# Env: MAC_HOST (user@address), MAC_SSH_OPTIONS (for ssh, such as -i <key>), LAB_SERVER (where this machine and the
#      Mac both reach the lab server; default http://127.0.0.1:2283, the tunnels of the mac-dev skill),
#      IMMICH_INSTANCE (as lab/immich-server takes it).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${MAC_HOST:?set MAC_HOST to the Mac user@address}"
server=${LAB_SERVER:-http://127.0.0.1:2283}
env_file="$here/../immich-server/.env${IMMICH_INSTANCE:+.$IMMICH_INSTANCE}.local"
# shellcheck source=/dev/null
. "$env_file"
# Relative to the home folder, where ssh starts: the location of the File Provider domain "Immich".
root='Library/CloudStorage/ImmichMount-Immich'

# shellcheck disable=SC2086
mac() { ssh ${MAC_SSH_OPTIONS:-} -o BatchMode=yes -o ConnectTimeout=10 "$MAC_HOST" "$1" </dev/null; }
failures=0
verdict() {
  if [ "$2" = "$3" ]; then echo "PASS $1"; else echo "FAIL $1: expected $3, got $2"; failures=$((failures + 1)); fi
}
# Wait up to a minute for a remote command to print the expected text.
wait_for_mac() {
  local name=$1 command=$2 expected=$3 seen=""
  for _ in $(seq 1 30); do
    seen=$(mac "$command" 2>/dev/null || true)
    [ "$seen" = "$expected" ] && break
    sleep 2
  done
  verdict "$name" "$seen" "$expected"
}
api() { curl -sS --fail-with-body -H "x-api-key: $IMMICH_API_KEY_FULL" "$@"; }

curl -sS -m 10 "$server/api/server/version" >/dev/null || { echo "the lab server does not answer at $server" >&2; exit 1; }
mac true || { echo "the Mac does not answer at $MAC_HOST" >&2; exit 1; }
started=$(date +%s)

verdict "the sections" "$(mac "ls $root | sort | tr '\n' ' '")" "Albums People Photos Places Upload "
verdict "every timeline photo" "$(mac "find $root/Photos -type f | wc -l | tr -d ' '")" "50"
verdict "listing downloads no original" "$(mac "ls -lOR $root/Photos | grep -c dataless")" "50"

# Byte for byte: each original read through Finder is one of the server's, by Immich's SHA-1.
checksums=$(api -X POST "$server/api/search/metadata" -H 'content-type: application/json' -d '{"size":1000}' |
  jq -r '.assets.items[].checksum')
read_back=$(mac "find $root/Photos -type f | sort | head -3 | while IFS= read -r file; do
  shasum -a 1 \"\$file\" | cut -c1-40 | xxd -r -p | base64; done")
matching=$(grep -c -x -F -f <(printf '%s\n' "$checksums") <<<"$read_back" || true)
verdict "originals read back as the server's" "$matching" "3"

thumbnail=$(mac "file=\$(find $root/Photos -type f | sort | sed -n 10p); rm -rf /tmp/immich-mount-check;
  mkdir -p /tmp/immich-mount-check; qlmanage -t -s 256 -o /tmp/immich-mount-check \"\$file\" >/dev/null 2>&1;
  printf '%s %s' \$(find /tmp/immich-mount-check -name '*.png' | wc -l) \$(ls -lO \"\$file\" | grep -c dataless);
  rm -rf /tmp/immich-mount-check")
verdict "a thumbnail, and its original still not downloaded" "$thumbnail" "1 1"

# A photo of January 2001, a year the library has none of, so a year and a month appear with it. Bytes appended to
# a fixture make it unique, so Immich does not take it for a duplicate.
photo=$(mktemp --suffix=.jpg)
cat "$here/../immich-server/fixtures/cache/001.jpg" >"$photo"
head -c 64 /dev/urandom >>"$photo"
asset=$(api -X POST "$server/api/assets" -F "assetData=@$photo;filename=\"MAC_CHECK.jpg\"" \
  -F "fileCreatedAt=2001-01-15T12:00:00.000Z" -F "fileModifiedAt=2001-01-15T12:00:00.000Z" | jq -r .id)
rm -f "$photo"
wait_for_mac "a photo added on the server shows" "find $root/Photos/2001 -type f 2>/dev/null | wc -l | tr -d ' '" "1"
api -X DELETE "$server/api/assets" -H 'content-type: application/json' -d "{\"ids\":[\"$asset\"],\"force\":true}" >/dev/null
wait_for_mac "a photo deleted on the server goes" "ls $root/Photos | grep -c '^2001\$'" "0"

echo "$failures failed, $(($(date +%s) - started)) s"
[ "$failures" -eq 0 ]
