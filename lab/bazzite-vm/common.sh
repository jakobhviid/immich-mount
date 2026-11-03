# Shared settings for the bazzite-vm scripts. Sourced, not run.
#
#   BAZZITE_VM  the test VM (default: whichever of ind-bazzite-gnome-1 and ind-bazzite-kde-1 exists)
#   EREBUS_DIR  path to the erebus repo (default: an erebus checkout next to this repository)
#
# The lab's projects share the default project's image store, where the lab key may add an image but not remove
# it, so a Bazzite edition is kept as a custom block volume in the lab project instead of an Incus image:
# build-image.sh writes ind-bazzite-<edition>-base, and up.sh boots a copy of it.
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
[ -f "$EREBUS_DIR/lib/erebus.sh" ] || { echo "no lib/erebus.sh in EREBUS_DIR=$EREBUS_DIR" >&2; exit 1; }
export EREBUS_DIR
# lib/erebus.sh gives us inc (the bin/incus wrapper), step, die, state, wait_for.
. "$EREBUS_DIR/lib/erebus.sh"
PROJECT=lab
POOL=local
TESTER=tester

# upstream_image <edition>: the published bootc image of a Bazzite edition. KDE Plasma is Bazzite's default.
upstream_image() {
  case $1 in
    gnome) echo ghcr.io/ublue-os/bazzite-gnome:stable ;;
    kde) echo ghcr.io/ublue-os/bazzite:stable ;;
    *) die "unknown edition '$1' (gnome or kde)" ;;
  esac
}
base_volume() { echo "ind-bazzite-$1-base"; }

# pick_vm: set VM to BAZZITE_VM, or to the one Bazzite test VM that exists.
pick_vm() {
  if [ -n "${BAZZITE_VM:-}" ]; then VM=$BAZZITE_VM; return; fi
  local candidates
  candidates=$(inc list --project "$PROJECT" -f csv -c n '^ind-bazzite-(gnome|kde)-1$' </dev/null)
  case $(printf '%s' "$candidates" | grep -c .) in
    1) VM=$candidates ;;
    0) die "no Bazzite VM (lab/bazzite-vm/up.sh gnome|kde)" ;;
    *) die "both editions are up; say which with BAZZITE_VM=ind-bazzite-gnome-1 or ind-bazzite-kde-1" ;;
  esac
}
# vm_edition: the edition up.sh recorded on the VM.
vm_edition() { inc config get --project "$PROJECT" "$VM" user.bazzite.edition </dev/null; }
# vm <cmd …>: run a command as root in the VM, never reading our stdin.
vm() { inc exec --project "$PROJECT" "$VM" -- "$@" </dev/null; }
