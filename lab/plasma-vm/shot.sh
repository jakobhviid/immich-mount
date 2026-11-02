#!/usr/bin/env bash
# Take a screenshot of the headless Plasma session and copy it here.
#
#   lab/plasma-vm/shot.sh /tmp/plasma.png
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
[ $# -eq 1 ] || die "usage: $0 <local.png>"
remote=/home/$TESTER/.shot.png
vm rm -f "$remote"
# Spectacle without its window: the whole screen, saved, no notification. KWin allows it because session.sh set
# KWIN_SCREENSHOT_NO_PERMISSION_CHECKS.
vm /usr/local/bin/in-session spectacle --background --nonotify --fullscreen --output "$remote" >/dev/null 2>&1 || true
vm test -s "$remote" || die "no screenshot (is the session up? lab/plasma-vm/session.sh)"
inc file pull --project "$P" "$VM$remote" "$1" </dev/null
step "screenshot -> $1"
