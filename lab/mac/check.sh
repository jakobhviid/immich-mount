#!/usr/bin/env bash
# The macOS edition's check, from this machine: Finder's view of the lab server on the maintainer's Mac, one PASS or
# FAIL line per claim (the mac-dev skill). It needs, already in place: a signed Debug build of the app in the Mac's
# checkout, pulled to this commit, the tunnels to the lab server, and the extension switched on. It starts fresh: the
# app signs out (the location, the credential and the store go) and in again with the lab's full key. It reads a few
# originals, adds one photo on the server and one through Upload, and deletes both from the server again.
#
#   lab/mac/check.sh
#
# Env: MAC_HOST (user@address), MAC_SSH_OPTIONS (for ssh, such as -i <key>), LAB_SERVER (where this machine and the
#      Mac both reach the lab server; default http://127.0.0.1:2283, the tunnels of the mac-dev skill, which the
#      check opens itself when nothing answers there, and closes at its end), IMMICH_INSTANCE (as lab/immich-server
#      takes it), EREBUS_DIR (for those tunnels; default the erebus checkout beside this one).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${MAC_HOST:?set MAC_HOST to the Mac user@address}"
server=${LAB_SERVER:-http://127.0.0.1:2283}
env_file="$here/../immich-server/.env${IMMICH_INSTANCE:+.$IMMICH_INSTANCE}.local"
# shellcheck source=/dev/null
. "$env_file"
# Relative to the home folder, where ssh starts: the location of the File Provider domain "Immich", and the checkout.
root='Library/CloudStorage/ImmichMount-Immich'
checkout='Developer/Private/immich-mount'
app="$checkout/apple/build/DerivedData/Build/Products/Debug/Immich Mount.app"

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

mac true || { echo "the Mac does not answer at $MAC_HOST" >&2; exit 1; }
# The tunnels of the mac-dev skill, for this run only: the lab server to this machine's loopback, and on to the Mac's.
if [ -z "${LAB_SERVER:-}" ] && ! curl -s -m 5 "$server/api/server/version" >/dev/null; then
  erebus=${EREBUS_DIR:-$(cd "$here/../../.." && pwd)/erebus}
  socat TCP-LISTEN:2283,bind=127.0.0.1,reuseaddr,fork \
    EXEC:"$erebus/bin/incus exec ${IMMICH_INSTANCE:-ind-immich-srv} -- socat STDIO TCP\:127.0.0.1\:2283" &
  tunnels=$!
  # shellcheck disable=SC2086
  ssh ${MAC_SSH_OPTIONS:-} -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -N \
    -R 127.0.0.1:2283:127.0.0.1:2283 "$MAC_HOST" </dev/null &
  tunnels="$tunnels $!"
  # shellcheck disable=SC2064
  trap "kill $tunnels 2>/dev/null" EXIT
  for _ in $(seq 1 10); do mac "curl -s -m 5 $server/api/server/version" >/dev/null && break; sleep 1; done
fi
curl -sS -m 10 "$server/api/server/version" >/dev/null || { echo "the lab server does not answer at $server" >&2; exit 1; }
started=$(date +%s)

since=$(mac "date '+%Y-%m-%d %H:%M:%S'")
mac "pkill -x 'Immich Mount'; sleep 1; open -n '$app' --args -signOut YES -signInServer $server \
  -signInAPIKey $IMMICH_API_KEY_FULL"
# The app logs its sign-in once the location is back; the old folder may still answer until then.
wait_for_mac "a fresh sign-in" "/usr/bin/log show --start '$since' --style compact \
  --predicate 'subsystem == \"dev.hviid.ImmichMount\"' | grep -c 'signed in; the Finder location is added'" "1"
wait_for_mac "the sections" "ls $root 2>/dev/null | sort | tr '\n' ' '" "Albums People Photos Places Upload "
verdict "the extension's status: reachable, with the server's version" \
  "$(mac "cat Library/Group\\ Containers/*.dev.hviid.ImmichMount/status.json" | jq -r '"\(.online) \(.version)"')" "true v3.3.5"
verdict "every timeline photo" "$(mac "find $root/Photos -type f | wc -l | tr -d ' '")" "50"
verdict "listing downloads no original" "$(mac "ls -lOR $root/Photos | grep -c dataless")" "50"

# Byte for byte: each original read through Finder is one of the server's, by Immich's SHA-1.
checksums=$(api -X POST "$server/api/search/metadata" -H 'content-type: application/json' -d '{"size":1000}' |
  jq -r '.assets.items[].checksum')
read_back=$(mac "find $root/Photos -type f | sort | head -3 | while IFS= read -r file; do
  openssl dgst -sha1 -binary \"\$file\" | base64; done")
matching=$(grep -c -x -F -f <(printf '%s\n' "$checksums") <<<"$read_back" || true)
verdict "originals read back as the server's" "$matching" "3"
videos=$(mac "find $root/Photos -type f \\( -iname '*.mp4' -o -iname '*.mov' -o -iname '*.webm' \\) | while IFS= read -r file; do
  openssl dgst -sha1 -binary \"\$file\" | base64; done")
verdict "every timeline video read back as the server's" "$(grep -c -x -F -f <(printf '%s\n' "$checksums") <<<"$videos" || true)" "4"

# Finder's way to a thumbnail (QLThumbnailGenerator), on a photo not downloaded; qlmanage reads the file instead.
thumbnail=$(mac "file=\$(find \$HOME/$root/Photos -type f -exec stat -f '%Sf|%N' {} + | grep '^[^|]*dataless' | head -1 | cut -d'|' -f2-);
  made=\$(cd $checkout && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift lab/mac/thumbnail.swift \"\$file\" | grep -c '^thumbnail ');
  printf '%s %s' \"\$made\" \$(ls -lO \"\$file\" | grep -c dataless)")
verdict "a thumbnail, and its original still not downloaded" "$thumbnail" "1 1"

# A photo added to an album on the server shows in it, and goes when taken out. Under the default layout the album's
# folder may change its year with the photo, so every folder of the album is counted.
album=$(api "$server/api/albums" | jq -r '.[] | select(.albumName=="Holiday Mix") | .id')
members=$(api -X POST "$server/api/search/metadata" -H 'content-type: application/json' -d "{\"albumIds\":[\"$album\"],\"size\":100}" |
  jq -r '.assets.items[].id')
outsider=$(api -X POST "$server/api/search/metadata" -H 'content-type: application/json' -d '{"size":1000}' |
  jq -r '.assets.items[].id' | grep -v -x -F -f <(printf '%s\n' "$members") | head -1)
in_album="find $root/Albums -path '*Holiday Mix/*' -type f | wc -l | tr -d ' '"
api -X PUT "$server/api/albums/$album/assets" -H 'content-type: application/json' -d "{\"ids\":[\"$outsider\"]}" >/dev/null
wait_for_mac "a photo added to an album on the server shows in it" "$in_album" "6"
api -X DELETE "$server/api/albums/$album/assets" -H 'content-type: application/json' -d "{\"ids\":[\"$outsider\"]}" >/dev/null
wait_for_mac "and goes when taken out" "$in_album" "5"

# A photo of January 2001, a year the library has none of, so a year and a month appear with it, added while the
# extension is stopped: the app wakes it, and its kept poll sees the change. Bytes appended to a fixture make the
# photo unique, so Immich does not take it for a duplicate.
mac "pkill -x ImmichMountFileProvider" || true
photo=$(mktemp --suffix=.jpg)
cat "$here/../immich-server/fixtures/cache/001.jpg" >"$photo"
head -c 64 /dev/urandom >>"$photo"
asset=$(api -X POST "$server/api/assets" -F "assetData=@$photo;filename=\"MAC_CHECK.jpg\"" \
  -F "fileCreatedAt=2001-01-15T12:00:00.000Z" -F "fileModifiedAt=2001-01-15T12:00:00.000Z" | jq -r .id)
rm -f "$photo"
wait_for_mac "a photo added on the server while the extension was stopped shows" "find $root/Photos/2001 -type f 2>/dev/null | wc -l | tr -d ' '" "1"
api -X DELETE "$server/api/assets" -H 'content-type: application/json' -d "{\"ids\":[\"$asset\"],\"force\":true}" >/dev/null
wait_for_mac "a photo deleted on the server goes" "ls $root/Photos | grep -c '^2001\$'" "0"

# A unique photo moved into Upload: the server has it byte for byte, its source is gone, and it leaves Upload.
photo=$(mktemp --suffix=.jpg)
cat "$here/../immich-server/fixtures/cache/002.jpg" >"$photo"
head -c 64 /dev/urandom >>"$photo"
checksum=$(openssl dgst -sha1 -binary "$photo" | base64)
mac "mkdir -p Developer/Private/immich-mount/apple/build/check" || true
# shellcheck disable=SC2086
scp -q ${MAC_SSH_OPTIONS:-} -o BatchMode=yes "$photo" "$MAC_HOST:Developer/Private/immich-mount/apple/build/check/MAC_UPLOAD.jpg"
rm -f "$photo"
mac "mv Developer/Private/immich-mount/apple/build/check/MAC_UPLOAD.jpg $root/Upload/"
verdict "the moved photo's source is gone" "$(mac "ls Developer/Private/immich-mount/apple/build/check | wc -l | tr -d ' '")" "0"
found=""
for _ in $(seq 1 30); do
  found=$(api -X POST "$server/api/search/metadata" -H 'content-type: application/json' \
    -d "{\"checksum\":\"$checksum\"}" | jq -r '.assets.items[0].id // empty')
  [ -n "$found" ] && break
  sleep 2
done
verdict "a photo moved into Upload reaches the server byte for byte" "$([ -n "$found" ] && echo yes || echo no)" "yes"
wait_for_mac "and leaves Upload" "ls $root/Upload | wc -l | tr -d ' '" "0"
# A file Immich does not take is refused, and stays in Upload rather than being lost; deleting it removes it.
mac "printf 'not a photo\\n' > Developer/Private/immich-mount/apple/build/check/notes.txt
  cp Developer/Private/immich-mount/apple/build/check/notes.txt $root/Upload/"
sleep 15
verdict "a file Immich does not take stays in Upload" "$(mac "ls $root/Upload | tr '\n' ' '")" "notes.txt "
mac "rm $root/Upload/notes.txt; rm -rf Developer/Private/immich-mount/apple/build/check"
if [ -n "$found" ]; then
  api -X DELETE "$server/api/assets" -H 'content-type: application/json' -d "{\"ids\":[\"$found\"],\"force\":true}" >/dev/null
fi

echo "$failures failed, $(($(date +%s) - started)) s"
[ "$failures" -eq 0 ]
