#!/usr/bin/env bash
# Boot a Bazzite VM on Erebus from a copy of an edition's base disk (build-image.sh): a stock Bazzite, with Homebrew as
# Bazzite sets it up and nothing of Immich Mount. Adds the first user, `tester` (uid 1000, in wheel, home
# /home/tester through Bazzite's /home -> var/home link), with linger, and the lab's in-VM helpers.
#
#   lab/bazzite-vm/up.sh [gnome|kde]       (default gnome; the VM is ind-bazzite-<edition>-1, or BAZZITE_VM)
#
# Next: lab/bazzite-vm/session.sh starts the headless GNOME or Plasma session.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
edition=${1:-gnome}
upstream_image "$edition" >/dev/null
VM=${BAZZITE_VM:-ind-bazzite-$edition-1}
case "$VM" in ind-bazzite-gnome-* | ind-bazzite-kde-*) ;; *) die "'$VM' is not an ind-bazzite-<edition>-* name" ;; esac
base=$(base_volume "$edition")
[ -z "$(state "$PROJECT" "$VM")" ] || die "$VM already exists (BAZZITE_VM=other-name, or down.sh first)"
built=$(inc storage volume get "$POOL" "$base" --project "$PROJECT" user.bazzite.version </dev/null 2>/dev/null) \
  || die "no $base volume: run lab/bazzite-vm/build-image.sh $edition"

# The VM's disk is a copy of the base (a btrfs snapshot, instant), named like the VM, and not kept: down.sh, or
# Erebus' teardown, deletes it. The instance's own root disk stays empty; the firmware boots Bazzite's disk first.
# Bazzite's kernel is signed with Universal Blue's key, which the VM's Secure Boot does not hold.
step "launching $VM: Bazzite $built ($edition), 4 vCPU, 8 GiB"
inc storage volume copy "$POOL/$base" "$POOL/$VM" --project "$PROJECT" </dev/null >/dev/null
inc storage volume unset "$POOL" "$VM" user.keep --project "$PROJECT" </dev/null
inc init --empty --vm "$VM" --project "$PROJECT" -c limits.cpu=4 -c limits.memory=8GiB \
  -c security.secureboot=false -c user.bazzite.edition="$edition" -d root,size=1GiB </dev/null >/dev/null
inc config device add --project "$PROJECT" "$VM" bazzite disk pool="$POOL" source="$VM" boot.priority=10 </dev/null >/dev/null
inc start --project "$PROJECT" "$VM" </dev/null

# Bazzite reboots itself once on its first boot (bazzite-hardware-setup adds kernel arguments, then "Rebooting to
# apply karg changes"), and Incus shows the VM STOPPED for a moment, which wait_for takes for a failure. Wait for it
# to come back, a bounded number of times, so a real power-off still fails.
wait_through_reboots() {
  local label=$1 restarts=0 deadline
  shift
  until wait_for "$PROJECT" "$VM" "$label" "$@"; do
    restarts=$((restarts + 1))
    [ "$restarts" -le 2 ] || die "$VM keeps stopping (inc console --project $PROJECT $VM --show-log)"
    step "$VM restarted itself; waiting for it to run again"
    deadline=$(( $(date +%s) + 120 ))
    until [ "$(state "$PROJECT" "$VM")" = RUNNING ]; do
      [ "$(date +%s)" -lt "$deadline" ] || die "$VM did not start again after stopping"
      sleep 2
    done
  done
}
wait_through_reboots "the Incus agent" true
wait_through_reboots "the boot to finish" sh -c \
  'case $(systemctl is-system-running) in running|degraded) exit 0 ;; *) exit 1 ;; esac'
reboots=$(vm journalctl --list-boots --no-pager -q | wc -l)
[ "$reboots" -le 1 ] || echo "         $VM booted $reboots times (bazzite-hardware-setup's first-boot reboot)"
step "booted: $(vm sh -c '. /etc/os-release; echo $PRETTY_NAME') $(vm uname -r)"
# ublue-os-media-automount fails on every boot here: a bootc install writes no /etc/fstab entries, which it expects.
failed=$(vm systemctl --failed --plain --no-legend | awk '{print $1}' | tr '\n' ' ')
[ -z "$failed" ] || echo "         failed units on this boot: $failed"

# Bazzite's own timers would update the system, Flatpaks and Homebrew during a check (uupd runs all three).
vm sh -c 'for timer in uupd.timer brew-update.timer brew-upgrade.timer rpm-ostreed-automatic.timer; do
  systemctl stop "$timer" 2>/dev/null || true; done'

# Bazzite's installer makes users with HOME=/home/<name>, through the link, which is what hides the mount from GVfs;
# plain useradd on Bazzite would give /var/home/<name> (its /etc/default/useradd), so the home is set explicitly.
vm sh -c "id $TESTER >/dev/null 2>&1 || useradd -m -d /home/$TESTER -G wheel $TESTER; loginctl enable-linger $TESTER"
uid=$(vm id -u "$TESTER")
[ "$uid" = 1000 ] || echo "         $TESTER has uid $uid, not 1000: Bazzite's Homebrew belongs to uid 1000"
wait_for "$PROJECT" "$VM" "user@$uid.service" systemctl is-active -q "user@$uid.service"
wait_for "$PROJECT" "$VM" "tester's session bus" test -S "/run/user/$uid/bus"

# The in-VM helpers of the Fedora VMs, which fit Bazzite unchanged: /usr/local and /opt are links into /var there.
inc file push --project "$PROJECT" "$here/../gnome-vm/in-session" "$VM/usr/local/bin/in-session" --mode 0755 </dev/null
case $edition in
  gnome) tar -C "$here/../gnome-vm/exp" -cf - . |
           inc exec --project "$PROJECT" "$VM" -- sh -c 'mkdir -p /opt/exp && tar -xmf - -C /opt/exp && chmod +x /opt/exp/*' ;;
  kde) inc file push --project "$PROJECT" "$here/../plasma-vm/act.py" "$VM/usr/local/bin/act.py" --mode 0755 </dev/null ;;
esac

brew_owner=$(vm stat -c %U /home/linuxbrew/.linuxbrew 2>/dev/null || echo "not set up")
step "ready: $VM (Homebrew: /home/linuxbrew/.linuxbrew, owner $brew_owner)"
