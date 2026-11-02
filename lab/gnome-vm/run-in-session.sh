#!/usr/bin/env bash
# Run a command as tester inside the headless GNOME session (session bus,
# XDG_RUNTIME_DIR and WAYLAND_DISPLAY set). Arguments pass through unchanged;
# use single quotes for ~ or $HOME so they expand inside the VM.
#
#   lab/gnome-vm/run-in-session.sh gio info -a 'filesystem::*' /home/tester/Immich
#   lab/gnome-vm/run-in-session.sh -d nautilus /home/tester/Immich   # detached GUI app
#   lab/gnome-vm/run-in-session.sh sh -c 'ls ~/.cache/thumbnails'
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
[ $# -gt 0 ] || die "usage: $0 [-d] <command …>"
vm /usr/local/bin/in-session "$@"
