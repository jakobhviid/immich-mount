#!/usr/bin/env bash
# Delete the Plasma VM. Refuses anything not named ind-plasma-*.
#
#   PLASMA_VM=ind-plasma-1 lab/plasma-vm/down.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
case "$VM" in ind-plasma-*) ;; *) die "refusing to delete '$VM': not an ind-plasma-* instance" ;; esac
if [ -n "$(state "$P" "$VM")" ]; then
  inc delete --project "$P" -f "$VM" </dev/null
  step "deleted $VM"
else
  step "$VM does not exist"
fi
