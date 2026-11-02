#!/usr/bin/env bash
# Delete the VM (GNOME_VM, default ind-gnome-1). Touches nothing else.
#
#   lab/gnome-vm/down.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
case "$VM" in ind-gnome-*) ;; *) die "refusing to delete '$VM': not an ind-gnome-* instance" ;; esac
if [ -n "$(state "$P" "$VM")" ]; then
  inc delete --project "$P" -f "$VM" </dev/null
  step "deleted $VM"
else
  step "$VM does not exist"
fi
