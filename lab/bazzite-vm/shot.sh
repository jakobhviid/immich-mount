#!/usr/bin/env bash
# Screenshot the whole headless session and pull it to <local.png>, the way the Fedora VM of the same desktop does.
#
#   lab/bazzite-vm/shot.sh /tmp/bazzite.png
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
pick_vm
case $(vm_edition) in
  gnome) GNOME_VM=$VM exec "$here/../gnome-vm/shot.sh" "$@" ;;
  kde) PLASMA_VM=$VM exec "$here/../plasma-vm/shot.sh" "$@" ;;
  *) die "$VM has no user.bazzite.edition (was it made by up.sh?)" ;;
esac
