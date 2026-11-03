#!/usr/bin/env bash
# Launch a fresh Fedora 44 VM on Erebus with KDE Plasma 6 (KWin, plasmashell, Dolphin), nothing of Immich Mount,
# and an unprivileged user `tester` with linger enabled: the M6 exit check installs the release packages into it.
#
#   PLASMA_VM=ind-plasma-1 lab/plasma-vm/up.sh [--fresh]
#
# A warm VM (one with its fresh snapshot) is restored to it; --fresh, or no snapshot, builds it.
#
# Next: lab/plasma-vm/session.sh starts a headless Plasma session.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
. "$here/../warm.sh"
build=false; [ "${1:-}" = --fresh ] && build=true
sweep_expired
if ! $build && has_fresh "$VM"; then
  step "restoring $VM to its fresh snapshot"
  restore_fresh "$VM"
else
if [ -n "$(state "$P" "$VM")" ]; then
  step "deleting $VM to build it again"
  inc delete --project "$P" -f "$VM" </dev/null
fi
# The same boot-entry trap as the GNOME VM (lab/gnome-vm/up.sh): fix /etc/kernel/cmdline before any kernel update.
userdata=$(cat <<'YAML'
#cloud-config
bootcmd:
  - [sh, -c, 'u=$(findmnt -no UUID /) && sed -i -E "s#root=[^ ]+#root=UUID=$u#" /etc/kernel/cmdline /boot/loader/entries/*.conf']
YAML
)
step "launching $VM (fedora/44/cloud VM, 4 vCPU, 8 GiB, 30 GiB)"
inc launch images:fedora/44/cloud "$VM" --vm --project "$P" -c user.expires="$(expires_at)" \
  -c limits.cpu=4 -c limits.memory=8GiB -d root,size=30GiB \
  -c cloud-init.user-data="$userdata" </dev/null
wait_cloud_init "$P" "$VM"
vm sh -c 'grep -q "root=UUID=" /etc/kernel/cmdline && ! grep -rq loop0p2 /etc/kernel/cmdline /boot/loader/entries' \
  || die "the root= trap fix did not apply (see /etc/kernel/cmdline)"
# Packages, weak dependencies on, as a Fedora KDE install would pull them in:
#   plasma-workspace, kwin, plasma-breeze, kde-settings-plasma   the session, its look and Fedora's KDE defaults
#   dolphin, kio-extras                                           the file manager under test, and its thumbnailer
#   kf6-kwallet                                                   ksecretd, KWallet's Secret Service
#   spectacle                                                     screenshots (shot.sh)
#   python3-gobject, at-spi2-core, python3-pyatspi                driving Qt apps through accessibility (act.py)
#   mesa-dri-drivers, google-noto-sans-fonts                      software rendering, and text in screenshots
#   fuse3, xdg-utils, dbus-tools                                  what the mount and the apps rely on
PKGS=(plasma-workspace kwin plasma-breeze kde-settings-plasma breeze-icon-theme dolphin kio-extras kf6-kwallet
      spectacle python3-gobject at-spi2-core python3-pyatspi mesa-dri-drivers google-noto-sans-fonts
      fuse3 xdg-utils dbus-tools)
step "dnf install (${#PKGS[@]} packages + deps)"
vm sh -c 'dnf -y install "$@" >/var/log/plasma-vm-dnf.log 2>&1 || { tail -20 /var/log/plasma-vm-dnf.log; exit 1; }' _ "${PKGS[@]}" \
  || die "dnf install failed (log: /var/log/plasma-vm-dnf.log in the VM)"
step "packages installed"
vm sh -c "id $TESTER >/dev/null 2>&1 || useradd -m $TESTER; loginctl enable-linger $TESTER"
step "taking its fresh snapshot"
snapshot_fresh "$VM"
fi
wait_for "$P" "$VM" "the VM agent" true
uid=$(vm id -u "$TESTER")
wait_for "$P" "$VM" "user@$uid.service" systemctl is-active -q "user@$uid.service"
wait_for "$P" "$VM" "tester's session bus" test -S "/run/user/$uid/bus"
# Pushed on every start, so a warm VM never runs old copies.
inc file push --project "$P" "$here/in-session" "$VM/usr/local/bin/in-session" --mode 0755 </dev/null
inc file push --project "$P" "$here/act.py" "$VM/usr/local/bin/act.py" --mode 0755 </dev/null
step "ready: $VM ($(vm sh -c '. /etc/os-release; echo $PRETTY_NAME') $(vm uname -r))"
