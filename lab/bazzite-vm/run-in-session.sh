#!/usr/bin/env bash
# Run a command as tester inside the headless session (session bus, XDG_RUNTIME_DIR and WAYLAND_DISPLAY set).
# Arguments pass through unchanged; use single quotes for ~ or $HOME so they expand inside the VM. Homebrew is on
# the PATH of a login shell, as in a terminal on Bazzite.
#
#   lab/bazzite-vm/run-in-session.sh bash -lc 'brew install jakobhviid/tap/immich-mount'
#   lab/bazzite-vm/run-in-session.sh -d nautilus /home/tester/Immich      # detached GUI app
#   lab/bazzite-vm/run-in-session.sh /opt/exp/input.sh key Return         # GNOME input (lab/gnome-vm/exp)
#   lab/bazzite-vm/run-in-session.sh act.py tree immich-mount-kde         # Plasma, by name (lab/plasma-vm/act.py)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
pick_vm
[ $# -gt 0 ] || die "usage: $0 [-d] <command …>"
vm /usr/local/bin/in-session "$@"
