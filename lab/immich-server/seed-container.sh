#!/usr/bin/env bash
# Runs INSIDE the Immich container; seed.sh pushes and calls it, one phase at a time:
#   setup      tools, admin sign-up + login, ML off, two API keys
#   upload     every manifest entry (renamed / stripped where its roles say so) + a generated MP4
#   idle       exit 0 when Immich's job queues are empty (polled by seed.sh via wait_for)
#   organize   albums, archive, trash, a manual person + face
#   summary    one-line verdict
# State (token, keys, uploaded ids) lives in /opt/immich/seed. Throwaway credentials only.
set -euo pipefail

API=http://127.0.0.1:2283/api
FIX=/opt/fixtures            # manifest.json + cache/, pushed by seed.sh
S=/opt/immich/seed
mkdir -p "$S"

# api METHOD PATH [JSON]: admin session call; prints the body, fails on HTTP >= 400.
api() {
  curl -sS --fail-with-body -X "$1" "$API$2" -H "Authorization: Bearer $(cat "$S/token")" \
    -H 'Content-Type: application/json' ${3:+--data "$3"}
}

# iso "2019:12:06 14:52:02" | "2024-07-18 07:30:27" | "2019-07-11"  ->  2019-12-06T14:52:02.000Z
# EXIF times carry no zone; sending them as UTC is good enough, Immich reads the EXIF itself.
iso() {
  local d
  d=$(echo "$1" | sed -E 's/^([0-9]{4})[:-]([0-9]{2})[:-]([0-9]{2})/\1-\2-\3/; s/ /T/')
  [ ${#d} -eq 10 ] && d="${d}T12:00:00"
  echo "${d}.000Z"
}

# upload FILE NAME DATE: multipart fields accepted by v3: assetData, fileCreatedAt,
# fileModifiedAt (deviceId/deviceAssetId no longer exist). The filename comes from the
# multipart part. Prints {"id":…,"status":"created"|"duplicate"}.
upload() {
  curl -sS --fail-with-body -X POST "$API/assets" -H "Authorization: Bearer $(cat "$S/token")" \
    -F "assetData=@$1;filename=\"$2\"" -F "fileCreatedAt=$3" -F "fileModifiedAt=$3"
}

case ${1:-} in
setup)
  export DEBIAN_FRONTEND=noninteractive
  apt-get install -y -qq jq libimage-exiftool-perl ffmpeg >/dev/null
  email=admin@example.com
  pass=$(head -c 300 /dev/urandom | tr -dc A-Za-z0-9 | cut -c1-20)   # no SIGPIPE under pipefail
  curl -sS --fail-with-body -X POST "$API/auth/admin-sign-up" -H 'Content-Type: application/json' \
    --data "{\"email\":\"$email\",\"password\":\"$pass\",\"name\":\"Lab Admin\"}" >/dev/null
  curl -sS --fail-with-body -X POST "$API/auth/login" -H 'Content-Type: application/json' \
    --data "{\"email\":\"$email\",\"password\":\"$pass\"}" | jq -r .accessToken > "$S/token"
  # No ML container: switch ML off so no smart-search/face jobs try to reach it.
  api GET /system-config | jq -c '.machineLearning.enabled = false' > "$S/config.json"
  api PUT /system-config "$(cat "$S/config.json")" >/dev/null
  full=$(api POST /api-keys '{"name":"lab-full","permissions":["all"]}' | jq -r .secret)
  ro=$(api POST /api-keys '{"name":"lab-readonly","permissions":["album.read","asset.read","asset.view","asset.download","person.read"]}' | jq -r .secret)
  printf 'IMMICH_ADMIN_EMAIL=%s\nIMMICH_ADMIN_PASSWORD=%s\nIMMICH_API_KEY_FULL=%s\nIMMICH_API_KEY_READONLY=%s\n' \
    "$email" "$pass" "$full" "$ro" > "$S/keys.env"
  echo "admin $email, ML disabled, keys lab-full + lab-readonly"
  ;;

upload)
  : > "$S/assets.tsv"   # manifestId  assetId  status  uploadName  roles
  jq -c '.files[]' "$FIX/manifest.json" | while read -r e; do
    id=$(jq -r .id <<<"$e"); name=$(jq -r .uploadName <<<"$e"); roles=$(jq -r '.roles|join(",")' <<<"$e")
    src="$FIX/cache/$(jq -r .cacheName <<<"$e")"
    date=$(iso "$(jq -r .dateTaken <<<"$e")")
    if [ ! -f "$src" ]; then   # only optional entries may be missing (seed.sh enforces it)
      echo "skip $id $name (not downloaded)"; continue
    fi
    work=/tmp/upload$(jq -r '.cacheName | sub("^[^.]*"; "")' <<<"$e")
    cp "$src" "$work"
    # Commons thumbnails carry no EXIF: write the original's pinned camera EXIF back
    # (DateTimeOriginal, OffsetTimeOriginal, Make, Model, GPS*). Empty for videos.
    mapfile -t tags < <(jq -r '.exif | to_entries[] | "-\(.key)=\(.value)"' <<<"$e")
    [ ${#tags[@]} -eq 0 ] || exiftool -q -overwrite_original "${tags[@]}" "$work"
    if [[ ,$roles, == *,strip-metadata,* ]]; then
      # The "no usable date" asset: no EXIF at all, and the epoch as the file date
      # (the upload API requires fileCreatedAt, so some date is always sent).
      exiftool -q -overwrite_original -all= "$work"
      date=1970-01-01T00:00:00.000Z
    fi
    r=$(upload "$work" "$name" "$date")
    printf '%s\t%s\t%s\t%s\t%s\n' "$id" "$(jq -r .id <<<"$r")" "$(jq -r .status <<<"$r")" "$name" "$roles" >> "$S/assets.tsv"
    rm -f "$work"
  done
  # Two generated H.264/AAC videos in the containers phones produce, with a QuickTime
  # creation_time (the Commons WebMs have none, and are optional). The MP4 also carries
  # a location atom (Copenhagen); the MOV has no location.
  gen() { # gen ID FILE CREATED [extra ffmpeg args…]
    local id=$1 file=$2 created=$3; shift 3
    ffmpeg -nostdin -loglevel error -y -f lavfi -i testsrc2=duration=4:size=1280x720:rate=30 \
      -f lavfi -i sine=frequency=440:duration=4 -c:v libx264 -pix_fmt yuv420p -c:a aac \
      -metadata creation_time="$created" "$@" -movflags +faststart "/tmp/$file"
    r=$(upload "/tmp/$file" "$file" "$created")
    printf '%s\t%s\t%s\t%s\t%s\n' "$id" "$(jq -r .id <<<"$r")" "$(jq -r .status <<<"$r")" "$file" "video,generated" >> "$S/assets.tsv"
    rm -f "/tmp/$file"
  }
  gen gen1 VID_20220615_100000.mp4 2022-06-15T10:00:00.000Z -metadata location=+55.6761+012.5683/
  gen gen2 IMG_4711.MOV 2023-07-04T18:30:00.000Z
  cut -f3 "$S/assets.tsv" | sort | uniq -c | tr '\n' ' '; echo "uploads"
  ;;

idle)
  api GET /queues | jq -e 'all(.[]; .statistics.active + .statistics.waiting + .statistics.delayed == 0)' >/dev/null
  ;;

organize)
  ids_with() { awk -F'\t' -v r="$1" '(","$5",") ~ ","r"," {print $2}' "$S/assets.tsv" | jq -R . | jq -sc .; }
  for album in "Holiday Mix" "Persons"; do   # "Persons" collides with the top-level folder name
    api POST /albums "$(jq -nc --arg n "$album" --argjson a "$(ids_with "album=$album")" '{albumName:$n, assetIds:$a}')" >/dev/null
  done
  api PUT /assets "$(jq -nc --argjson a "$(ids_with archive)" '{ids:$a, visibility:"archive"}')" >/dev/null
  api DELETE /assets "$(jq -nc --argjson a "$(ids_with trash)" '{ids:$a, force:false}')" >/dev/null
  # Persons normally come from ML face detection. Try the manual route: a named person
  # plus a hand-drawn face box on a photo of people.
  person=$(api POST /people '{"name":"Lab Person"}' | jq -r .id)
  asset=$(awk -F'\t' '$1=="028"{print $2}' "$S/assets.tsv")
  if api POST /faces "$(jq -nc --arg p "$person" --arg a "$asset" \
       '{personId:$p, assetId:$a, imageWidth:2448, imageHeight:3264, x:1000, y:900, width:400, height:400}')" >/dev/null; then
    echo "person $person with a manual face on asset $asset"
  else
    echo "person $person created; manual face FAILED"
  fi
  echo "albums, archive, trash done"
  ;;

summary)
  stats=$(api GET /assets/statistics)
  trashed=$(api GET '/assets/statistics?isTrashed=true' | jq .total)
  archived=$(api GET '/assets/statistics?visibility=archive' | jq .total)
  echo "assets=$(jq .total <<<"$stats") images=$(jq .images <<<"$stats") videos=$(jq .videos <<<"$stats")" \
       "archived=$archived trashed=$trashed albums=$(api GET /albums | jq length)" \
       "people=$(api GET /people | jq .total) countries=$(api GET '/search/suggestions?type=country' | jq -c .)"
  ;;

*) echo "usage: $0 setup|upload|idle|organize|summary" >&2; exit 2 ;;
esac
