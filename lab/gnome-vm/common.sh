# Shared settings for the gnome-vm scripts. Sourced, not run.
#
#   GNOME_VM   instance name (default ind-gnome-1)
#   EREBUS_DIR path to the erebus repo (default: an erebus checkout next to this repository)

EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
[ -f "$EREBUS_DIR/lib/erebus.sh" ] || { echo "no lib/erebus.sh in EREBUS_DIR=$EREBUS_DIR" >&2; exit 1; }
# lib/erebus.sh gives us inc (the bin/incus wrapper), step, die, wait_for, wait_cloud_init.
. "$EREBUS_DIR/lib/erebus.sh"

P=lab
VM=${GNOME_VM:-ind-gnome-1}
TESTER=tester

# vm <cmd …>: run a command as root in the VM, never reading our stdin.
vm() { inc exec --project "$P" "$VM" -- "$@" </dev/null; }
