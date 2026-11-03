#!/usr/bin/env bash
# Delete a Bazzite test VM and its copy of the base disk. The base volumes (build-image.sh) stay.
#
#   lab/bazzite-vm/down.sh            (BAZZITE_VM, or the one ind-bazzite-<edition>-1 that exists)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
pick_vm
case "$VM" in
  *-base) die "refusing to delete '$VM': a base volume's name" ;;
  ind-bazzite-gnome-* | ind-bazzite-kde-*) ;;
  *) die "refusing to delete '$VM': not an ind-bazzite-<edition>-* instance" ;;
esac
if [ -n "$(state "$PROJECT" "$VM")" ]; then
  inc delete --project "$PROJECT" -f "$VM" </dev/null
  step "deleted $VM"
else
  step "$VM does not exist"
fi
if [ -n "$(inc storage volume list "$POOL" --project "$PROJECT" -f csv -c n "^$VM\$" </dev/null)" ]; then
  inc storage volume delete "$POOL" "$VM" --project "$PROJECT" </dev/null
  step "deleted its disk, volume $VM"
fi
