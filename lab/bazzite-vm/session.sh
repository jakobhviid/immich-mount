#!/usr/bin/env bash
# Start the headless session of the VM's edition for tester, exactly as the Fedora VMs start theirs: GNOME Shell with
# a virtual monitor (lab/gnome-vm/session.sh), or KWin's virtual backend with plasmashell (lab/plasma-vm/session.sh).
# Idempotent: restarts the session.
#
#   lab/bazzite-vm/session.sh [WIDTHxHEIGHT]      (default 1280x800 on GNOME, 1440x900 on Plasma)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
pick_vm
case $(vm_edition) in
  gnome) GNOME_VM=$VM exec "$here/../gnome-vm/session.sh" "$@" ;;
  kde) PLASMA_VM=$VM exec "$here/../plasma-vm/session.sh" "$@" ;;
  *) die "$VM has no user.bazzite.edition (was it made by up.sh?)" ;;
esac
