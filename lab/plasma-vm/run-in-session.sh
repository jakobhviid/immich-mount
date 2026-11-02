#!/usr/bin/env bash
# Run a command as tester inside the headless Plasma session (session bus, XDG_RUNTIME_DIR and WAYLAND_DISPLAY set).
# Arguments pass through unchanged; use single quotes for ~ or $HOME so they expand inside the VM.
#
#   lab/plasma-vm/run-in-session.sh -d immich-mount-kde                      # detached GUI app
#   lab/plasma-vm/run-in-session.sh act.py press immich-mount-kde Continue   # press a button by its name
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
[ $# -gt 0 ] || die "usage: $0 [-d] <command …>"
vm /usr/local/bin/in-session "$@"
