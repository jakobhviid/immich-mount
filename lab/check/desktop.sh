#!/usr/bin/env bash
# The release check inside a desktop VM. Each check states a claim of SPEC.md, WORKFLOWS.md or ARCHITECTURE.md and
# prints one line: `PASS <name>: <evidence>`, `FAIL <name>: <what was seen>`, or `SKIP <name>: <why it does not
# apply>`. Exits 1 if any check failed.
#
#   desktop.sh <expected version> [--app-setup]
#
#   --app-setup  the app's first run did the setup, not `immich-mount login` and `mount`: on KDE, Dolphin's Places
#                must then hold the mount, which only that first run adds
#
# Runs as the desktop's user inside its session (lab/check/run.sh runs it through the VM kind's run-in-session.sh),
# with Immich Mount installed, signed in and mounted at ~/Immich. With a server.env beside it (IMMICH_URL and
# IMMICH_API_KEY_READONLY), it also asks the server about the copies and the upload. It does what a user would: copies
# into a scratch folder, one upload into Upload (and a paste into Photos refused), an unmount and a mount, and opens
# and closes the app.
set -o pipefail

expected_version=${1:?usage: desktop.sh <expected version> [--app-setup]}
app_setup=false
[ "${2:-}" = --app-setup ] && app_setup=true
here=$(cd "$(dirname "$0")" && pwd)
[ -f "$here/server.env" ] && . "$here/server.env"

# A terminal on Bazzite has Homebrew's programs at the end of its PATH (/etc/profile.d/brew.sh, interactive shells
# only); this script runs without one.
[ -d /home/linuxbrew/.linuxbrew/bin ] && PATH=$PATH:/home/linuxbrew/.linuxbrew/bin
# A desktop session's umask: copies take their modes through it.
umask 022
# A terminal in the session has the desktop's variables, which the session (a login, or the lab's session.sh) gave
# the user manager; run-in-session.sh does not pass them. doctor and the app read XDG_CURRENT_DESKTOP.
eval "$(systemctl --user show-environment 2>/dev/null |
  grep -E '^(XDG_CURRENT_DESKTOP|XDG_SESSION_TYPE|XDG_SESSION_CLASS|XDG_DATA_DIRS|KDE_FULL_SESSION|KDE_SESSION_VERSION)=' |
  sed 's/^/export /')"

failures=0
pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; failures=$((failures + 1)); }
skip() { echo "SKIP $1: $2"; }

user=$(id -un)
mount_point=$HOME/Immich
resolved_mount=$(realpath -m "$mount_point")
work=$(mktemp -d)
sitter=
cleanup() {
  [ -n "$sitter" ] && kill "$sitter" 2>/dev/null
  systemctl --user stop immich-check-app.service 2>/dev/null
  # Copies with read-only modes (the failure the copies check looks for) cannot be deleted until they are writable.
  chmod -R u+w "$work" 2>/dev/null
  rm -rf "$work"
}
trap cleanup EXIT

if pgrep -u "$user" -x plasmashell >/dev/null; then desktop=kde
elif pgrep -u "$user" -x gnome-shell >/dev/null; then desktop=gnome
else desktop=none
fi

# json_fields <name>=<dotted path>...: the fields of the JSON document on stdin as shell assignments, for eval.
# Booleans come out as true and false, and anything missing as an empty string.
json_fields() {
  python3 -c '
import json, shlex, sys
data = json.load(sys.stdin)
for argument in sys.argv[1:]:
    name, path = argument.split("=", 1)
    value = data
    for key in path.split("."):
        value = value.get(key) if isinstance(value, dict) else None
    if isinstance(value, bool):
        value = "true" if value else "false"
    print(name + "=" + shlex.quote("" if value is None else str(value)))
' "$@"
}

status_fields() { timeout 30 immich-mount status --json 2>/dev/null | json_fields "$@"; }

# find_data_file <relative path>: the first one in the XDG data directories, the order the desktop looks them up in.
find_data_file() {
  local directory IFS=:
  for directory in "${XDG_DATA_HOME:-$HOME/.local/share}" ${XDG_DATA_DIRS:-/usr/local/share:/usr/share}; do
    if [ -e "$directory/$1" ]; then echo "$directory/$1"; return 0; fi
  done
  return 1
}

is_mounted() { findmnt -n --mountpoint "$resolved_mount" >/dev/null 2>&1; }

# seconds_since <$EPOCHREALTIME of the start>: the time since, to a tenth of a second.
seconds_since() { awk -v start="$1" -v now="$EPOCHREALTIME" 'BEGIN { printf "%.1f", now - start }'; }

# wait_until <seconds> <command...>: poll every second; fails once the time is up.
wait_until() {
  local deadline=$((SECONDS + $1))
  shift
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 1
  done
}

# The server's assets with the file's content: "<count> <localDateTime of the first>", or nothing without server.env.
server_assets_like() {
  [ -n "${IMMICH_URL:-}" ] && [ -n "${IMMICH_API_KEY_READONLY:-}" ] || return 1
  local checksum
  checksum=$(python3 -c 'import base64, hashlib, sys; print(base64.b64encode(hashlib.sha1(open(sys.argv[1], "rb").read()).digest()).decode())' "$1")
  curl -fsS -m 20 -H "x-api-key: $IMMICH_API_KEY_READONLY" -H 'content-type: application/json' \
    -d "{\"checksum\":\"$checksum\"}" "$IMMICH_URL/api/search/metadata" |
    python3 -c 'import json, sys; items = json.load(sys.stdin)["assets"]["items"]; print(len(items), items[0]["localDateTime"] if items else "-")'
}

check_version() {
  local program reported
  program=$(command -v immich-mount) || { fail version "no immich-mount on the PATH ($PATH)"; return; }
  reported=$(immich-mount --version 2>&1 | head -1)
  if [ "${reported##* }" = "$expected_version" ]; then
    pass version "$reported, $program"
  else
    fail version "$program says '$reported', expected $expected_version"
  fi
}

# The one launcher entry (ARCHITECTURE.md, "Packaging and releases"), whose Exec runs the installed version.
launcher_command=
check_launcher() {
  local entry exec_line program reported
  entry=$(find_data_file applications/dev.hviid.ImmichMount.desktop) || { fail launcher "no dev.hviid.ImmichMount.desktop in the XDG data directories"; return; }
  exec_line=$(sed -n 's/^Exec=//p' "$entry" | head -1)
  program=$(command -v "${exec_line%% *}") || { fail launcher "$entry: Exec=$exec_line, and ${exec_line%% *} is no program"; return; }
  reported=$("$program" --version 2>&1 | head -1)
  launcher_command=$(printf '%s' "$exec_line" | sed -E 's/ ?%[a-zA-Z]//g')
  if [ "${reported##* }" = "$expected_version" ]; then
    pass launcher "$entry: Exec=$exec_line ($reported)"
  else
    fail launcher "$entry: Exec=$exec_line runs '$reported', expected $expected_version"
  fi
}

# The launcher registers the app.immich scheme, so single sign-on in the browser hands back to the app (WORKFLOWS.md).
check_scheme() {
  local handlers
  handlers=$(gio mime x-scheme-handler/app.immich 2>&1)
  if printf '%s\n' "$handlers" | grep -q '^[[:space:]]dev\.hviid\.ImmichMount\.desktop$'; then
    pass scheme "x-scheme-handler/app.immich: $(printf '%s\n' "$handlers" | head -1 | sed 's/^.*: //')"
  else
    fail scheme "dev.hviid.ImmichMount.desktop is not registered for x-scheme-handler/app.immich: $(printf '%s' "$handlers" | head -1)"
  fi
}

check_icons() {
  local icon path found=() missing=()
  for icon in scalable/apps/dev.hviid.ImmichMount.svg symbolic/apps/dev.hviid.ImmichMount-symbolic.svg; do
    if path=$(find_data_file "icons/hicolor/$icon") && grep -q '<svg' "$path"; then found+=("$path"); else missing+=("$icon"); fi
  done
  if [ ${#missing[@]} -eq 0 ]; then pass icons "${found[*]}"; else fail icons "missing or not SVG: ${missing[*]}"; fi
}

# The translations are where the programs look for them, beside the program (share/locale) or in ../share/locale, and
# the launcher has its comment in Danish (ARCHITECTURE.md, "Translations").
check_translations() {
  local program folder catalog="" entry comment
  program=$(readlink -f "$(command -v immich-mount)") || { fail translations "no immich-mount on the PATH"; return; }
  folder=$(dirname "$program")
  for candidate in "$folder/share/locale" "$folder/../share/locale"; do
    [ -f "$candidate/da/LC_MESSAGES/immich-mount.mo" ] && { catalog=$(readlink -f "$candidate/da/LC_MESSAGES/immich-mount.mo"); break; }
  done
  entry=$(find_data_file applications/dev.hviid.ImmichMount.desktop) || entry=
  comment=$(grep -m1 '^Comment\[da\]=' "${entry:-/dev/null}" | cut -d= -f2-)
  if [ -n "$catalog" ] && [ -n "$comment" ]; then
    local base count
    base=$(dirname "$(dirname "$(dirname "$catalog")")")
    count=$(ls "$base"/*/LC_MESSAGES/immich-mount.mo | wc -l)
    pass translations "$catalog, $count languages; Comment[da]=$comment"
  else
    fail translations "Danish catalog beside $program: ${catalog:-none}; Comment[da] in the launcher: ${comment:-none}"
  fi
}

# Signed in and online (WORKFLOWS.md, "Status").
check_status() {
  local state detail mounted path stale email credential version feed
  eval "$(status_fields state=state detail=detail mounted=mount.mounted path=mount.path stale=mount.stale \
    email=account.email credential=account.credential version=server.version feed=change_feed)"
  if [ "$state" = online ] && [ "$mounted" = true ] && [ "$stale" = false ] && [ -n "$email" ]; then
    pass status "online as $email ($credential), Immich $version, changes by $feed"
  else
    fail status "state ${state:-none}${detail:+ ($detail)}, mounted ${mounted:-?}, stale ${stale:-?}, account ${email:-none}"
  fi
}

# The user service is on and running (WORKFLOWS.md, "The mount": it starts at login).
check_service() {
  local enabled active unit_file
  enabled=$(systemctl --user is-enabled immich-mount.service 2>&1)
  active=$(systemctl --user is-active immich-mount.service 2>&1)
  unit_file=$(systemctl --user show -P FragmentPath immich-mount.service 2>/dev/null)
  if [ "$enabled" = enabled ] && [ "$active" = active ]; then
    pass service "immich-mount.service enabled and active, $unit_file"
  else
    fail service "immich-mount.service is-enabled $enabled, is-active $active ($unit_file)"
  fi
}

check_mounted() {
  local filesystem name
  filesystem=$(findmnt -n -o FSTYPE --mountpoint "$resolved_mount" 2>/dev/null)
  name=$(sed -n 's/^Name=//p' "$mount_point/.xdg-volume-info" 2>/dev/null)
  if [ "${filesystem#fuse}" != "$filesystem" ] && [ "$name" = Immich ]; then
    pass mounted "$filesystem at $resolved_mount, named $name"
  else
    fail mounted "at $resolved_mount: filesystem '${filesystem:-none}', volume name '${name:-none}'"
  fi
}

# GNOME: Files lists the mount as a drive with an eject button, on an atomic desktop too, where the daemon gives
# GVfs's volume monitor the resolved home (ARCHITECTURE.md, "GNOME integration"). KDE: Dolphin's Places hold it once
# the KDE app's first run added it (SPEC.md, "Files on disk").
check_sidebar() {
  case $desktop in
    gnome)
      local listing entry drop_in=
      listing=$(gio mount -li 2>&1)
      # The mount's own block: from its "Mount(n): Immich -> file://..." line to the next unindented line.
      entry=$(printf '%s\n' "$listing" | awk -v first="file://$mount_point" -v second="file://$resolved_mount" '
        /^[^ ]/ { inside = 0 }
        /^ *Mount\(/ { inside = (index($0, "-> " first) || index($0, "-> " second)) }
        inside')
      [ -f "$HOME/.config/systemd/user/gvfs-udisks2-volume-monitor.service.d/immich-mount.conf" ] && drop_in=", through the GVfs drop-in"
      if [ -z "$entry" ]; then
        fail sidebar "gio mount -li lists no mount of $mount_point"
      elif printf '%s\n' "$entry" | grep -q 'can_eject=1\|can_unmount=1'; then
        pass sidebar "a drive in Files: $(printf '%s\n' "$entry" | head -1 | sed 's/^ *//'), $(printf '%s\n' "$entry" | grep -o 'can_unmount=1\|can_eject=1' | paste -sd' ')$drop_in"
      else
        fail sidebar "gio mount -li lists it, but neither ejectable nor unmountable: $(printf '%s' "$entry" | tr '\n' ' ')"
      fi
      ;;
    kde)
      local places=${XDG_DATA_HOME:-$HOME/.local/share}/user-places.xbel
      if grep -q "<bookmark href=\"file://$mount_point\">" "$places" 2>/dev/null; then
        pass places "Dolphin's Places hold file://$mount_point ($places)"
      elif $app_setup; then
        fail places "no place for file://$mount_point in $places"
      else
        skip places "only the KDE app's first run adds the place, and this setup signed in from the command line"
      fi
      ;;
    *) skip sidebar "no GNOME Shell or Plasma session" ;;
  esac
}

# Two months of Photos: one to list and peek at (three files or more), another with a JPEG to copy. Listing a month
# seeds its thumbnails, so the search stops as soon as both are found.
peek_month=
copy_month=
choose_months() {
  local year month_directory files
  while IFS= read -r year; do
    for month_directory in "$mount_point/Photos/$year"/*/; do
      month_directory=${month_directory%/}
      files=$(find "$month_directory" -maxdepth 1 -type f 2>/dev/null | wc -l)
      if [ -z "$peek_month" ] && [ "$files" -ge 3 ]; then
        peek_month=$month_directory
      elif [ -z "$copy_month" ] && find "$month_directory" -maxdepth 1 -type f -iname '*.jpg' 2>/dev/null | grep -q .; then
        copy_month=$month_directory
      fi
      [ -n "$peek_month" ] && [ -n "$copy_month" ] && return 0
    done
  done < <(ls -1 "$mount_point/Photos" 2>/dev/null | sort -r)
  [ -n "$peek_month" ] && [ -n "$copy_month" ]
}

# access() is what the mount allows: no writing at its root, in Photos, its folders, Albums or to a photo; only
# Upload takes new files (SPEC.md, "Attributes").
check_access() {
  local photo year_directory entry path expected seen label wrong=() allowed=() refused=()
  photo=$(find "$peek_month" -maxdepth 1 -type f | head -1)
  year_directory=$(dirname "$peek_month")
  for entry in "$mount_point:FALSE" "$mount_point/Albums:FALSE" "$photo:FALSE" \
    "$mount_point/Photos:FALSE" "$year_directory:FALSE" "$peek_month:FALSE" "$mount_point/Upload:TRUE"; do
    expected=${entry##*:}
    path=${entry%:*}
    label=${path#"$mount_point"/}
    [ "$path" = "$mount_point" ] && label="the root"
    seen=$(gio info -a access::can-write "$path" 2>/dev/null | sed -n 's/^ *access::can-write: //p')
    if [ "$seen" != "$expected" ]; then
      wrong+=("$label can-write ${seen:-unknown}")
    elif [ "$expected" = TRUE ]; then
      allowed+=("$label")
    else
      refused+=("$label")
    fi
  done
  if [ ${#wrong[@]} -eq 0 ]; then
    pass access "writable: $(printf '%s, ' "${allowed[@]}" | sed 's/, $//'); not: $(printf '%s, ' "${refused[@]}" | sed 's/, $//')"
  else
    fail access "$(IFS=';'; echo "${wrong[*]}")"
  fi
}

# Thumbnails seeded on the first listing: x-large for every file, and on KDE normal and large too; xx-large follows
# in the background (SPEC.md, "Thumbnails"). The cache file is the MD5 of the file's URI as GLib writes it, which
# `gio info` prints; where home is reached through a link, under both paths (ARCHITECTURE.md, "Seeding").
check_thumbnails() {
  local file home uris=() buckets=x-large homes report
  [ "$desktop" = kde ] && buckets=x-large,normal,large
  homes=("$mount_point")
  [ "$resolved_mount" != "$mount_point" ] && homes+=("$resolved_mount")
  for file in "$peek_month"/*; do
    [ -f "$file" ] || continue
    for home in "${homes[@]}"; do
      uris+=("$(gio info "$home${file#"$mount_point"}" 2>/dev/null | sed -n 's/^uri: //p')")
    done
  done
  report=$(thumbnail_report "$buckets" "${uris[@]}")
  if [ -n "$report" ]; then
    fail thumbnails "${peek_month#"$mount_point"/}: $report"
    return
  fi
  # The sharp ones come from Immich's previews, in the background.
  if ! wait_until 40 bash -c '[ -z "$(thumbnail_report xx-large "$@")" ]' _ "${uris[@]}"; then
    fail thumbnails "${peek_month#"$mount_point"/}: $(thumbnail_report xx-large "${uris[@]}") (40 s after listing)"
    return
  fi
  pass thumbnails "${peek_month#"$mount_point"/}: ${buckets//,/, } and xx-large for ${#uris[@]} URIs (${homes[*]})"
}

# thumbnail_report <buckets> <uri>...: nothing when every URI has a valid thumbnail in every bucket (mode 0600, its
# Thumb::URI the URI), else what is wrong with the first that has not.
thumbnail_report() {
  python3 - "$@" <<'PYTHON'
import hashlib, os, struct, sys
buckets, uris = sys.argv[1].split(","), sys.argv[2:]
cache = os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "thumbnails")
if not uris or "" in uris:
    print("no URI for some files (gio info failed)")
    sys.exit()
for bucket in buckets:
    for uri in uris:
        path = os.path.join(cache, bucket, hashlib.md5(uri.encode()).hexdigest() + ".png")
        try:
            data = open(path, "rb").read()
        except OSError:
            print(f"no {bucket} thumbnail for {uri}")
            sys.exit()
        texts, position = {}, 8
        while position + 8 <= len(data):
            length, kind = struct.unpack(">I4s", data[position:position + 8])
            if kind == b"tEXt":
                key, _, value = data[position + 8:position + 8 + length].partition(b"\0")
                texts[key.decode()] = value.decode("latin-1")
            position += 12 + length
        mode = os.stat(path).st_mode & 0o777
        if texts.get("Thumb::URI") != uri or mode != 0o600:
            print(f"{path}: Thumb::URI {texts.get('Thumb::URI')}, mode {mode:o}")
            sys.exit()
PYTHON
}
export -f thumbnail_report

# A peek is cheap: each file's first 8 KiB and last 4 KiB, as rom-properties reads every file of a folder its file
# manager shows, cost the file's start and end (two small range requests, kept), not the original (ARCHITECTURE.md,
# "Peeks are served from the edges"). The page cache is dropped first, so the reads reach the daemon.
check_peek() {
  local files=() before after count growth
  mapfile -t files < <(find "$peek_month" -maxdepth 1 -type f | sort)
  count=${#files[@]}
  python3 -c '
import os, sys
for path in sys.argv[1:]:
    descriptor = os.open(path, os.O_RDONLY)
    os.posix_fadvise(descriptor, 0, 0, os.POSIX_FADV_DONTNEED)
    os.close(descriptor)
' "${files[@]}"
  eval "$(status_fields before=fetched.bytes)"
  python3 -c '
import os, sys
for path in sys.argv[1:]:
    size = os.stat(path).st_size
    descriptor = os.open(path, os.O_RDONLY)
    os.pread(descriptor, 8192, 0)
    os.pread(descriptor, 4096, max(0, size - 4096))
    os.close(descriptor)
' "${files[@]}"
  eval "$(status_fields after=fetched.bytes)"
  if [ -z "$before" ] || [ -z "$after" ]; then
    fail peek "status --json has no fetched.bytes (before '$before', after '$after')"
    return
  fi
  growth=$((after - before))
  if [ "$growth" = 0 ]; then
    # Nothing reached the server: the bytes were in memory or in the content cache, so nothing was measured.
    fail peek "$count files peeked, nothing fetched: no read reached the server, so the check measured nothing"
  elif [ "$growth" -lt $((count * 100000)) ]; then
    pass peek "$count files peeked, $((growth / 1000)) KB fetched ($((growth / count / 1000)) KB per file)"
  else
    fail peek "$count files peeked, $((growth / 1000)) KB fetched ($((growth / count / 1000)) KB per file, over 100)"
  fi
}

# Copies out are ordinary: a photo copied by GIO (as Files copies) and by cp, and an album folder by cp -r, come out
# writable, files 0644 and folders 0755 (SPEC.md, "What the mount refuses"), with the server's bytes.
copied_photo=
check_copies() {
  local photo album file_modes folder_modes files problems=() matches
  photo=$(find "$copy_month" -maxdepth 1 -type f -iname '*.jpg' | sort | head -1)
  album=$(for directory in "$mount_point/Albums"/*/; do
      printf '%s\t%s\n' "$(find "$directory" -type f | wc -l)" "${directory%/}"
    done | sort -n | awk -F'\t' '$1 > 0 { print $2; exit }')
  gio copy "$photo" "$work/gio-copy.jpg" 2>"$work/gio-copy.err" || problems+=("gio copy failed: $(head -1 "$work/gio-copy.err")")
  cp "$photo" "$work/cp-copy.jpg" || problems+=("cp failed")
  cp -r "$album" "$work/album" || problems+=("cp -r of ${album#"$mount_point"/} failed")
  file_modes=$( { stat -c %a "$work/gio-copy.jpg" "$work/cp-copy.jpg"; find "$work/album" -type f -printf '%m\n'; } 2>/dev/null | sort -u | paste -sd/)
  folder_modes=$(find "$work/album" -type d -printf '%m\n' 2>/dev/null | sort -u | paste -sd/)
  files=$(( $(find "$work/album" -type f 2>/dev/null | wc -l) + 2 ))
  [ "$file_modes" = 644 ] && [ "$folder_modes" = 755 ] || problems+=("file modes ${file_modes:-none}, folder modes ${folder_modes:-none}")
  cmp -s "$photo" "$work/cp-copy.jpg" && cmp -s "$photo" "$work/gio-copy.jpg" || problems+=("a copy differs from the mount's file")
  if matches=$(server_assets_like "$work/cp-copy.jpg"); then
    [ "${matches%% *}" -ge 1 ] || problems+=("no asset on the server has the copied photo's SHA-1")
    matches=", its SHA-1 the server's"
  else
    matches=", not compared with the server (no server.env)"
  fi
  if [ ${#problems[@]} -eq 0 ]; then
    copied_photo=$work/cp-copy.jpg
    pass copies "${photo#"$mount_point"/} (gio copy, cp) and ${album#"$mount_point"/} (cp -r): $files files $file_modes, the folder $folder_modes$matches"
  else
    [ -f "$work/cp-copy.jpg" ] && copied_photo=$work/cp-copy.jpg
    fail copies "${photo#"$mount_point"/}, ${album#"$mount_point"/}: $(IFS=';'; echo "${problems[*]}")"
  fi
}

# An upload: a photo the server does not have, copied into Upload, uploads on close; its name stays as a receipt,
# and the asset appears in its date folder once Immich has read its metadata and the change feed has turned
# (WORKFLOWS.md, "Uploading"). The photo is the one just copied out, with a JPEG comment that makes its bytes new; its
# EXIF date is its source's, so its date folder is the source's month. Until Immich reads the metadata, the asset
# carries the upload's time, and shows in that month for a moment.
check_upload() {
  [ -n "$copied_photo" ] || { fail upload "no photo to upload (the copies failed)"; return; }
  local name size started elapsed matches before
  name=immich-check-$(date +%Y%m%d-%H%M%S).jpg
  python3 - "$copied_photo" "$work/upload.jpg" "Immich Mount release check $(date -u +%FT%T.%NZ)" <<'PYTHON'
import sys
data = open(sys.argv[1], "rb").read()
# After the APPn segments (EXIF among them), where readers expect metadata to start.
position = 2
while data[position] == 0xFF and 0xE0 <= data[position + 1] <= 0xEF:
    position += 2 + int.from_bytes(data[position + 2:position + 4], "big")
comment = sys.argv[3].encode()
segment = b"\xff\xfe" + (len(comment) + 2).to_bytes(2, "big") + comment
open(sys.argv[2], "wb").write(data[:position] + segment + data[position:])
PYTHON
  size=$(stat -c %s "$work/upload.jpg")
  if matches=$(server_assets_like "$work/upload.jpg") && [ "${matches%% *}" != 0 ]; then
    fail upload "the server has the new photo before it was copied ($matches)"
    return
  fi
  before=$(ls -1 "$copy_month" 2>/dev/null)
  started=$EPOCHREALTIME
  if ! cp "$work/upload.jpg" "$mount_point/Upload/$name" 2>"$work/upload.err"; then
    fail upload "cp into Upload failed: $(head -1 "$work/upload.err")"
    return
  fi
  elapsed=$(seconds_since "$started")
  [ -e "$mount_point/Upload/$name" ] || { fail upload "Upload/$name is not there as a receipt after the copy"; return; }
  if matches=$(server_assets_like "$work/upload.jpg"); then
    if [ "${matches%% *}" -lt 1 ]; then
      fail upload "cp returned 0 in $elapsed s, but no asset on the server has its SHA-1"
      return
    fi
    pass upload "Upload/$name ($((size / 1000)) KB): cp returned in $elapsed s, the server has its SHA-1, the receipt is in Upload"
  else
    pass upload "Upload/$name ($((size / 1000)) KB): cp returned in $elapsed s, the receipt is in Upload (not asked of the server: no server.env)"
  fi
  # A new name in the source's month, with the upload's size.
  if wait_until 90 sh -c '
      for file in "$1"/*; do
        [ -f "$file" ] && [ "$(stat -c %s "$file")" = "$2" ] && ! printf "%s\n" "$3" | grep -qxF "$(basename "$file")" && exit 0
      done
      exit 1' _ "$copy_month" "$size" "$before"; then
    matches=$(server_assets_like "$work/upload.jpg")
    pass upload-listed "in ${copy_month#"$mount_point"/} $(seconds_since "$started") s after the copy began${matches:+, taken ${matches#* } on the server}"
  else
    fail upload-listed "nothing new of $size bytes in ${copy_month#"$mount_point"/} 90 s after the copy${matches:+ (the server: ${matches#* })}"
  fi
  # Once Immich has it, the file leaves Upload by itself, a few seconds after its upload (SPEC.md, "Uploads").
  if wait_until 30 test ! -e "$mount_point/Upload/$name"; then
    pass upload-gone "Upload/$name left Upload $(seconds_since "$started") s after the copy began"
  else
    fail upload-gone "Upload/$name is still in Upload 30 s after the copy"
  fi
}

# The library's folders are read-only views: a paste beside a photo is refused, nothing is uploaded, and nothing is
# left behind (SPEC.md, "What the mount refuses").
check_read_only_views() {
  local photo target error
  photo=$(find "$copy_month" -maxdepth 1 -type f -name '*.jpg' | head -1)
  target="$copy_month/pasted beside its source.jpg"
  if cp "$photo" "$target" 2>"$work/paste.err"; then
    fail read-only "a paste into ${copy_month#"$mount_point"/} succeeded"
    return
  fi
  error=$(head -1 "$work/paste.err")
  case $error in
    *"Read-only file system"*) ;;
    *) fail read-only "a paste into ${copy_month#"$mount_point"/} failed, but not as read-only: $error"; return ;;
  esac
  [ ! -e "$target" ] || { fail read-only "the refused paste left ${target#"$mount_point"/} behind"; return; }
  pass read-only "a paste beside ${photo#"$mount_point"/} refused: Read-only file system"
}

# doctor: no problem and no warning (WORKFLOWS.md, "Doctor").
check_doctor() {
  local report
  report=$(timeout 60 immich-mount doctor --json 2>/dev/null | python3 -c '
import json, sys
data = json.load(sys.stdin)
others = ["%s %s: %s" % (check["name"], check["status"], check["detail"]) for check in data["checks"] if check["status"] not in ("ok", "skipped")]
print(len(data["checks"]), data["problems"], data["warnings"], "; ".join(others))')
  if [ -z "$report" ]; then
    fail doctor "doctor --json printed no report"
  elif [ "$(echo "$report" | cut -d' ' -f2,3)" = "0 0" ]; then
    pass doctor "$(echo "$report" | cut -d' ' -f1) checks, no problem, no warning"
  else
    fail doctor "$(echo "$report" | cut -d' ' -f2) problems, $(echo "$report" | cut -d' ' -f3) warnings: $(echo "$report" | cut -d' ' -f4-)"
  fi
}

# The launcher's command opens the interface native to the desktop: Kirigami on Plasma, GTK elsewhere (SPEC.md,
# "The immich-mount command"). Started as the desktop starts an app, in the user manager; still running after 5 s
# means its interface and libraries loaded.
check_app() {
  local interface process
  [ -n "$launcher_command" ] || { fail app "no launcher command to run"; return; }
  case $desktop in
    kde) interface=immich-mount-kde ;;
    gnome) interface=immich-mount-gnome ;;
    *) skip app "no GNOME Shell or Plasma session"; return ;;
  esac
  # comm holds 15 characters.
  process=${interface:0:15}
  systemctl --user stop immich-check-app.service 2>/dev/null
  systemd-run --user --collect --quiet --unit=immich-check-app -- sh -c "exec $launcher_command"
  if ! wait_until 15 pgrep -u "$user" -x "$process" >/dev/null; then
    fail app "$launcher_command started no $interface within 15 s ($(journalctl --user -u immich-check-app -n 2 -o cat 2>/dev/null | tr '\n' ' '))"
  else
    sleep 5
    if pgrep -u "$user" -x "$process" >/dev/null; then
      pass app "$launcher_command opened $interface, still running after 5 s"
    else
      fail app "$interface stopped within 5 s ($(journalctl --user -u immich-check-app -n 2 -o cat 2>/dev/null | tr '\n' ' '))"
    fi
  fi
  systemctl --user stop immich-check-app.service 2>/dev/null
}

# Unmount with a process sitting in the mount: it detaches lazily, and the daemon stops (SPEC.md, "The immich-mount
# command"; WORKFLOWS.md, "The mount").
check_unmount() {
  local output code
  (cd "$mount_point/Photos" && exec sleep 600) &
  sitter=$!
  sleep 1
  output=$(timeout 60 immich-mount unmount 2>&1)
  code=$?
  wait_until 15 sh -c '! systemctl --user is-active -q immich-mount.service'
  if [ "$code" = 0 ] && ! is_mounted && ! systemctl --user is-active -q immich-mount.service; then
    pass unmount "exit 0 with a process inside Photos; detached, the service $(systemctl --user is-active immich-mount.service)"
  else
    fail unmount "exit $code ($(printf '%s' "$output" | tail -1)); mounted: $(is_mounted && echo yes || echo no), the service $(systemctl --user is-active immich-mount.service)"
  fi
  kill "$sitter" 2>/dev/null
  wait "$sitter" 2>/dev/null
  sitter=
}

check_remount() {
  local output code state
  if is_mounted; then
    skip remount "still mounted: the unmount did not take it out"
    return
  fi
  # GVfs keeps the old volume through a quick unmount and mount; wait for it to go first.
  [ "$desktop" = gnome ] && wait_until 20 sh -c '! gio mount -l 2>/dev/null | grep -qF -- "-> file://$1"' _ "$resolved_mount"
  output=$(timeout 60 immich-mount mount 2>&1)
  code=$?
  wait_until 30 sh -c 'immich-mount status --json 2>/dev/null | grep -q "\"state\": *\"online\""'
  eval "$(status_fields state=state)"
  if [ "$code" = 0 ] && is_mounted && [ "$state" = online ]; then
    pass remount "immich-mount mount: exit 0, mounted, $state"
  else
    fail remount "exit $code ($(printf '%s' "$output" | tail -1)); mounted: $(is_mounted && echo yes || echo no), state ${state:-none}"
  fi
}

check_version
check_launcher
check_scheme
check_icons
check_translations
check_status
check_service
check_mounted
check_sidebar
if choose_months; then
  check_access
  check_thumbnails
  check_peek
  check_copies
  check_upload
  check_read_only_views
else
  fail folders "no month with three files and another with a JPEG under $mount_point/Photos (found '${peek_month#"$mount_point"/}' and '${copy_month#"$mount_point"/}')"
fi
check_doctor
check_app
check_unmount
check_remount
exit $((failures > 0))
