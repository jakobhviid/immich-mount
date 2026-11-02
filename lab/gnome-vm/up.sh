#!/usr/bin/env bash
# Launch a Fedora 44 VM on Erebus with a minimal GNOME (shell + Nautilus), the
# test tools for the FUSE/thumbnail experiments, and an unprivileged user
# `tester` with linger enabled. Prints timings.
#
#   GNOME_VM=ind-gnome-1 lab/gnome-vm/up.sh
#
# Next: lab/gnome-vm/session.sh starts the headless GNOME Shell.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

[ -z "$(state "$P" "$VM")" ] || die "$VM already exists (GNOME_VM=other-name, or down.sh first)"

# Trap (erebus docs/traps.md): the fedora/44/cloud image has root=/dev/loop0p2 in
# /etc/kernel/cmdline, which kernel-install (20-grub.install) copies into the boot
# entry of every NEW kernel, so the first kernel update leaves an unbootable
# default entry. The image (2026-11) no longer ships grubby, so fix the file with
# sed, plus any BLS entry that spells root= out (the shipped one uses $kernelopts).
userdata=$(cat <<'EOF'
#cloud-config
bootcmd:
  - [sh, -c, 'u=$(findmnt -no UUID /) && sed -i -E "s#root=[^ ]+#root=UUID=$u#" /etc/kernel/cmdline /boot/loader/entries/*.conf']
EOF
)

step "launching $VM (fedora/44/cloud VM, 4 vCPU, 8 GiB, 30 GiB)"
inc launch images:fedora/44/cloud "$VM" --vm --project "$P" \
  -c limits.cpu=4 -c limits.memory=8GiB -d root,size=30GiB \
  -c cloud-init.user-data="$userdata" </dev/null
wait_cloud_init "$P" "$VM"
step "cloud-init done"
vm sh -c 'grep -q "root=UUID=" /etc/kernel/cmdline && ! grep -rq loop0p2 /etc/kernel/cmdline /boot/loader/entries' \
  || die "the root= trap fix did not apply (see /etc/kernel/cmdline)"

# Packages. Weak deps stay on, as on a real Fedora Workstation, so we see the
# thumbnailers and indexer (localsearch) that GNOME would really pull in.
#   gnome-shell, nautilus     the session and the file manager under test
#   mesa-dri-drivers          llvmpipe software rendering (no GPU in the VM)
#   glycin-loaders, glycin-thumbnailer, gdk-pixbuf2   GNOME 50 image loading/thumbnailing
#   adwaita-*-fonts           so text renders in screenshots
#   fuse3, bindfs             a FUSE mount (fuse.bindfs) standing in for ours
#   inotify-tools, bpftrace   which file is opened, and by which process (fatrace/fanotify
#                             cannot see accesses through a non-allow_other FUSE mount)
#   ImageMagick, perl-Image-ExifTool, python3-pillow   fixtures and seeded thumbnails
PKGS=(gnome-shell nautilus mesa-dri-drivers glycin-loaders glycin-thumbnailer gdk-pixbuf2
      adwaita-sans-fonts adwaita-mono-fonts localsearch
      fuse3 bindfs inotify-tools bpftrace strace ImageMagick perl-Image-ExifTool python3-pillow
      glib2 dbus-tools xdg-user-dirs)
step "dnf install (${#PKGS[@]} packages + deps)"
vm sh -c 'dnf -y install "$@" >/var/log/gnome-vm-dnf.log 2>&1 || { tail -20 /var/log/gnome-vm-dnf.log; exit 1; }' _ "${PKGS[@]}" \
  || die "dnf install failed (log: /var/log/gnome-vm-dnf.log in the VM)"
step "packages installed"

# The test user. Linger starts its systemd --user manager (and session bus at
# /run/user/<uid>/bus) at boot, with no login needed.
vm sh -c "id $TESTER >/dev/null 2>&1 || useradd -m $TESTER; loginctl enable-linger $TESTER"
uid=$(vm id -u "$TESTER")
wait_for "$P" "$VM" "user@$uid.service" systemctl is-active -q "user@$uid.service"
wait_for "$P" "$VM" "tester's session bus" test -S "/run/user/$uid/bus"

# The in-VM helper that run-in-session.sh, session.sh and shot.sh use, and the
# experiment helpers (exp/) at /opt/exp. -m: the VM clock may lag ours.
inc file push --project "$P" "$here/in-session" "$VM/usr/local/bin/in-session" --mode 0755 </dev/null
tar -C "$here/exp" -cf - . | inc exec --project "$P" "$VM" -- sh -c 'mkdir -p /opt/exp && tar -xmf - -C /opt/exp && chmod +x /opt/exp/*'

step "ready: $VM ($(vm sh -c '. /etc/os-release; echo $PRETTY_NAME') $(vm uname -r))"
