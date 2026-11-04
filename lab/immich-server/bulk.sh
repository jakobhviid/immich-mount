#!/usr/bin/env bash
# Add a large library of generated photos to the lab server, to measure how folders of many files behave: a month
# of hundreds of photos, a year of twelve months. The pictures are ffmpeg's test pattern (testsrc2), so there are no
# personal photos and no licences to track; each month's pictures get a width of their own, from the month (even:
# odd widths are rounded), so no two are the same file and Immich keeps them all.
#
#   lab/immich-server/bulk.sh 1997-01:400 1997-02:400 1998:10
#
# A month (YYYY-MM:count) gets that many photos, a year (YYYY:count) that many in each of its twelve months. Waits
# until Immich has made their thumbnails. Not part of the seed: up.sh restores the seeded snapshot, without them, so
# run it again after each up.sh. Env: IMMICH_INSTANCE (default ind-immich-srv), EREBUS_DIR.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
NAME=${IMMICH_INSTANCE:-ind-immich-srv}
P=lab
# shellcheck source=/dev/null
. "$EREBUS_DIR/lib/erebus.sh"

[ $# -gt 0 ] || die "usage: $0 YYYY-MM:count|YYYY:count ..."
[ "$(state "$P" "$NAME")" = RUNNING ] || die "$NAME is not running; run ./up.sh and ./seed.sh first"

months=()
for wanted in "$@"; do
  period=${wanted%%:*} count=${wanted##*:}
  [[ $count =~ ^[0-9]+$ ]] || die "not a count: $wanted"
  if [[ $period =~ ^[0-9]{4}$ ]]; then
    for month in 01 02 03 04 05 06 07 08 09 10 11 12; do months+=("$period-$month:$count"); done
  elif [[ $period =~ ^[0-9]{4}-[0-9]{2}$ ]]; then
    months+=("$period:$count")
  else
    die "not a month or a year: $wanted"
  fi
done

step "generating and uploading ${#months[@]} months of photos in $NAME"
inc exec --project "$P" "$NAME" -- bash -s -- "${months[@]}" <<'IN_CONTAINER'
set -euo pipefail
API=http://127.0.0.1:2283/api
token=$(cat /opt/immich/seed/token)
work=/tmp/bulk
rm -rf "$work"
for wanted in "$@"; do
  month=${wanted%%:*} count=${wanted##*:}
  mkdir -p "$work/$month"
  # The width tells the months apart, 25 years of them; testsrc2 tells the frames of one month apart.
  width=$((1700 + 2 * ((10#${month%-*} * 12 + 10#${month#*-}) % 300)))
  ffmpeg -nostdin -loglevel error -f lavfi -i "testsrc2=size=${width}x1200:rate=1" -frames:v "$count" -q:v 4 \
    "$work/$month/%05d.jpg"
done
# One capture time per photo, unique within its month: day, hour, minute and second from its number.
upload() {
  local path=$1 month number
  month=$(basename "$(dirname "$path")") number=$((10#$(basename "$path" .jpg) - 1))
  local taken
  taken=$(printf '%s-%02dT%02d:%02d:%02d.000Z' "$month" $((number % 28 + 1)) $((number / 28 % 24)) \
    $((number / 672 % 60)) $((number % 60)))
  curl -sS --fail-with-body -X POST "$API/assets" -H "Authorization: Bearer $token" \
    -F "assetData=@$path;filename=\"BULK_${month}_$(basename "$path")\"" -F "fileCreatedAt=$taken" \
    -F "fileModifiedAt=$taken" | jq -r .status
}
export -f upload
export API token
find "$work" -name '*.jpg' | sort | xargs -P 8 -I{} bash -c 'upload "$1"' _ {} | sort | uniq -c | tr '\n' ' '
echo
rm -rf "$work"
IN_CONTAINER
wait_for "$P" "$NAME" "job queues idle (metadata, thumbnails)" bash /opt/immich/seed-container.sh idle
step "done"
