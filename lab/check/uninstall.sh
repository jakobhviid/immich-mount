#!/usr/bin/env bash
# After `brew uninstall --cask --zap jakobhviid/tap/immich-mount`, inside the desktop VM's session as its user: waits
# for the daemon's own cleanup, which follows about two and a half minutes after its program is gone (ARCHITECTURE.md,
# "The daemon's lifecycle"), then reports what is left, one line each: `PASS <name>: <evidence>` where it is what
# WORKFLOWS.md ("Disconnect and uninstall") and the cask's zap say, `FAIL <name>: <what is left>`. Exits 1 if any
# check failed.
#
#   uninstall.sh [<seconds to wait for the daemon to leave, default 300>]
set -o pipefail

patience=${1:-300}
# A terminal on Bazzite has Homebrew's programs at the end of its PATH; this script runs without one.
[ -d /home/linuxbrew/.linuxbrew/bin ] && PATH=$PATH:/home/linuxbrew/.linuxbrew/bin

failures=0
pass() { echo "PASS $1: $2"; }
fail() { echo "FAIL $1: $2"; failures=$((failures + 1)); }

user=$(id -un)
mount_point=$HOME/Immich
resolved_mount=$(realpath -m "$mount_point")
user_units=$HOME/.config/systemd/user
trash=${XDG_DATA_HOME:-$HOME/.local/share}/Trash
daemon_running() { pgrep -u "$user" -f 'immich-mount daemon' >/dev/null; }
is_mounted() { findmnt -n --mountpoint "$resolved_mount" >/dev/null 2>&1; }
# trashed <path>: the Trash's entry for what was at that path, by its .trashinfo (the name gets a number when taken).
trashed() {
  local info
  info=$(grep -lx "Path=$1" "$trash"/info/*.trashinfo 2>/dev/null | head -1)
  [ -n "$info" ] && echo "$trash/files/$(basename "$info" .trashinfo)"
}

left=()
for program in immich-mount immich-mount-gnome immich-mount-kde; do
  command -v "$program" >/dev/null && left+=("$(command -v "$program")")
done
if [ ${#left[@]} -eq 0 ]; then pass program "no immich-mount, immich-mount-gnome or immich-mount-kde on the PATH"; else fail program "still there: ${left[*]}"; fi

left=()
for file in applications/dev.hviid.ImmichMount.desktop icons/hicolor/scalable/apps/dev.hviid.ImmichMount.svg \
  icons/hicolor/symbolic/apps/dev.hviid.ImmichMount-symbolic.svg; do
  [ -e "${XDG_DATA_HOME:-$HOME/.local/share}/$file" ] && left+=("$file")
done
gio mime x-scheme-handler/app.immich 2>/dev/null | grep -q 'dev\.hviid\.ImmichMount\.desktop' && left+=("the app.immich handler")
if [ ${#left[@]} -eq 0 ]; then pass launcher "the launcher entry, both icons and the app.immich handler are gone"; else fail launcher "still there: ${left[*]}"; fi

# Open in Immich's menu items, which the cask puts in the home folder, and the folder of Dolphin's plugin.
left=()
for item in "$HOME/.local/share/nautilus-python/extensions/immich-mount-nautilus.py" "$HOME/.local/lib/immich-mount" \
  "$HOME/.config/plasma-workspace/env/immich-mount.sh"; do
  [ -e "$item" ] && left+=("$item")
done
if [ ${#left[@]} -eq 0 ]; then pass menus "Files' and Dolphin's menu items and Dolphin's login script are gone"; else fail menus "still there: ${left[*]}"; fi

# The daemon looks every 30 seconds, leaves once its program has been gone for two minutes, and cleans up as it goes.
started=$SECONDS
while daemon_running && [ $((SECONDS - started)) -lt "$patience" ]; do sleep 5; done
if daemon_running; then
  fail process "the daemon still runs $patience s after the uninstall: $(pgrep -u "$user" -af 'immich-mount daemon' | head -1)"
else
  pass process "no daemon, $((SECONDS - started)) s after the uninstall"
fi

if is_mounted; then fail mount "still mounted at $resolved_mount"; else pass mount "nothing mounted at $resolved_mount"; fi

state=$(systemctl --user is-active immich-mount.service 2>&1)
enabled=$(systemctl --user is-enabled immich-mount.service 2>&1)
left=()
[ "$state" = active ] || [ "$state" = failed ] && left+=("is-active $state")
[ "$enabled" = enabled ] && left+=("is-enabled $enabled")
[ -e "$user_units/immich-mount.service" ] && left+=("$user_units/immich-mount.service")
[ -L "$user_units/default.target.wants/immich-mount.service" ] && left+=("the enable link in default.target.wants")
if [ ${#left[@]} -eq 0 ]; then
  pass unit "is-active $state, is-enabled $enabled; no unit file, no enable link"
else
  fail unit "${left[*]}"
fi

drop_in=$user_units/gvfs-udisks2-volume-monitor.service.d/immich-mount.conf
if [ -e "$drop_in" ]; then fail gvfs-drop-in "still there: $drop_in"; else pass gvfs-drop-in "no $drop_in"; fi

left=()
for uri in "file://$mount_point" "file://$resolved_mount"; do
  grep -qF "$uri" "$HOME/.config/gtk-3.0/bookmarks" 2>/dev/null && left+=("a bookmark of $uri")
  grep -qF "<bookmark href=\"$uri\">" "${XDG_DATA_HOME:-$HOME/.local/share}/user-places.xbel" 2>/dev/null && left+=("a place for $uri")
done
if [ ${#left[@]} -eq 0 ]; then pass sidebar "no bookmark in Files' sidebar, no place in Dolphin's"; else fail sidebar "${left[*]}"; fi

# Seeded thumbnails are small copies of private photos in a shared folder: any whose Thumb::URI is on the mount.
seeded=$(python3 - "file://$mount_point/" "file://$resolved_mount/" <<'PYTHON'
import glob, os, struct, sys
cache = os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "thumbnails")
count, buckets = 0, set()
for path in glob.glob(os.path.join(cache, "*", "*.png")):
    data = open(path, "rb").read()
    position = 8
    while position + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[position:position + 8])
        if kind == b"tEXt":
            key, _, value = data[position + 8:position + 8 + length].partition(b"\0")
            if key == b"Thumb::URI" and value.decode("latin-1").startswith(tuple(sys.argv[1:])):
                count += 1
                buckets.add(os.path.basename(os.path.dirname(path)))
        position += 12 + length
print(count, ",".join(sorted(buckets)))
PYTHON
)
if [ "${seeded%% *}" = 0 ]; then pass thumbnails "no thumbnail of a file on the mount"; else fail thumbnails "${seeded%% *} thumbnails of files on the mount, in ${seeded#* }"; fi

# The zap moves the caches and the settings to the Trash; trashed is fine, deleted too.
for entry in "cache:$HOME/.cache/immich-mount" "settings:$HOME/.config/immich-mount"; do
  name=${entry%%:*}
  path=${entry#*:}
  if [ -e "$path" ]; then
    fail "$name" "still there: $path"
  elif [ -n "$(trashed "$path")" ]; then
    pass "$name" "$path is in the Trash ($(trashed "$path"))"
  else
    pass "$name" "$path is gone"
  fi
done

# The credential stays: an uninstall may be a reinstall, and `logout` is what removes it (WORKFLOWS.md). In the
# keyring, or, with credential.file, in the settings the zap moved to the Trash.
kept=
if command -v secret-tool >/dev/null && timeout 10 secret-tool search application dev.hviid.ImmichMount 2>/dev/null | grep -q .; then
  kept="in the keyring"
fi
settings=$(trashed "$HOME/.config/immich-mount")
[ -n "$settings" ] && [ -f "$settings/credential" ] && kept="${kept:+$kept, and }in the Trash with the settings ($settings/credential)"
# Where nothing moved it (an uninstall without --zap).
[ -f "$HOME/.config/immich-mount/credential" ] && kept="${kept:+$kept, and }in $HOME/.config/immich-mount/credential"
if [ -n "$kept" ]; then pass credential "kept, $kept"; else fail credential "neither in the keyring nor in the Trash"; fi

exit $((failures > 0))
