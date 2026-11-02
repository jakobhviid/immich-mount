#!/usr/bin/env bash
# Screenshot the whole headless session and pull it to <local.png>.
# Uses org.gnome.Shell.Screenshot, which the shell only allows in --unsafe-mode.
#
#   lab/gnome-vm/shot.sh /tmp/nautilus.png
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
[ $# -eq 1 ] || die "usage: $0 <local.png>"
remote=/home/$TESTER/.shot.png
vm rm -f "$remote"
# Screenshot(include_cursor, flash, filename) -> (success, filename_used)
out=$(vm /usr/local/bin/in-session gdbus call --session --dest org.gnome.Shell.Screenshot \
  --object-path /org/gnome/Shell/Screenshot --method org.gnome.Shell.Screenshot.Screenshot \
  false false "$remote")
case "$out" in "(true,"*) ;; *) die "screenshot failed: $out" ;; esac
inc file pull --project "$P" "$VM$remote" "$1" </dev/null
step "screenshot -> $1"
