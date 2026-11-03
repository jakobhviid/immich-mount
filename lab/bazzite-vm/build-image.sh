#!/usr/bin/env bash
# Build the Bazzite disks that up.sh boots, one per edition: the published bootc image with the Incus agent's units
# added (so `incus exec` works, and nothing else), installed by its own bootc onto a custom block volume,
# ind-bazzite-<edition>-base, in the lab project. Slow and rarely needed: run it to test a newer Bazzite.
#
#   lab/bazzite-vm/build-image.sh [gnome] [kde]      (default: both)
#
# Nothing of Bazzite runs on this machine. A disposable Fedora 44 VM on Erebus (ind-bazzite-build, ephemeral) pulls
# the images, builds and installs them, and is deleted at the end (BAZZITE_KEEP_BUILDER=1 keeps it, to debug). The
# volumes carry user.keep=true, and what was found in the image (Bazzite and Fedora versions, GTK, libadwaita, Qt,
# fuse3, Homebrew) as user.bazzite.* keys, which up.sh prints.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
editions=("$@")
[ ${#editions[@]} -gt 0 ] || editions=(gnome kde)
for edition in "${editions[@]}"; do upstream_image "$edition" >/dev/null; done
BUILDER=${BAZZITE_BUILDER:-ind-bazzite-build}
TARGET_DISK=/dev/disk/by-id/scsi-0QEMU_QEMU_HARDDISK_incus_target
builder() { inc exec --project "$PROJECT" "$BUILDER" -- "$@" </dev/null; }

remove_builder() {
  [ "${BAZZITE_KEEP_BUILDER:-}" = 1 ] && { echo "kept $BUILDER (ephemeral: stopping it deletes it)" >&2; return; }
  [ -z "$(state "$PROJECT" "$BUILDER")" ] || { inc delete --project "$PROJECT" -f "$BUILDER" </dev/null && step "deleted $BUILDER"; }
}
trap remove_builder EXIT

if [ -z "$(state "$PROJECT" "$BUILDER")" ]; then
  step "launching $BUILDER (fedora/44/cloud VM, ephemeral, 8 vCPU, 16 GiB, 120 GiB)"
  inc launch images:fedora/44/cloud "$BUILDER" --vm --ephemeral --project "$PROJECT" \
    -c limits.cpu=8 -c limits.memory=16GiB -d root,size=120GiB </dev/null
fi
wait_cloud_init "$PROJECT" "$BUILDER"
# Incus' Fedora image runs SELinux in the kernel with no policy loaded, so every process is "kernel", which bootc's
# install cannot parse ("context_new() failed"). With the module off, as on the Ubuntu hosts Universal Blue builds
# its disks on, bootc labels the files from Bazzite's own policy. grub reads kernelopts from the ESP's grub.cfg here
# (/boot/grub2/grubenv is not read), and a reboot keeps the ephemeral VM.
if builder grep -q selinux /sys/kernel/security/lsm; then
  step "rebooting $BUILDER with selinux=0"
  builder sed -i -E '/^  set kernelopts=/ s/"$/ selinux=0"/' /boot/efi/EFI/fedora/grub.cfg
  builder systemctl reboot || true
  wait_for "$PROJECT" "$BUILDER" "the reboot with selinux=0" sh -c '! grep -q selinux /sys/kernel/security/lsm'
fi
builder sh -c 'command -v podman >/dev/null || dnf -y install podman >/var/log/build-dnf.log 2>&1' \
  || die "dnf install podman failed (/var/log/build-dnf.log in $BUILDER)"

# The derived image: Incus' own images ship these three files, and the agent itself arrives on a drive at boot.
# /usr is read-only on the booted system, so they go in at build time. Bazzite enforces SELinux, where Incus' images
# do not: the agent, copied to /run, is var_run_t and runs as init_t, which may not listen on vsock ("Failed to
# listen on vsock: permission denied"). Labelled as a program (bin_t), systemd runs it as unconfined_service_t.
builder sh -c 'set -e; mkdir -p /root/lab-image && cd /root/lab-image
  cp /usr/lib/systemd/system/incus-agent.service /usr/lib/systemd/incus-agent-setup \
     /usr/lib/udev/rules.d/99-incus-agent.rules .
  printf "[Service]\nExecStartPre=/usr/bin/chcon -t bin_t /run/incus_agent/incus-agent\n" > 10-selinux.conf
  cat > Containerfile <<EOF
ARG BASE
FROM \${BASE}
COPY incus-agent.service /usr/lib/systemd/system/incus-agent.service
COPY 10-selinux.conf /usr/lib/systemd/system/incus-agent.service.d/10-selinux.conf
COPY incus-agent-setup /usr/lib/systemd/incus-agent-setup
COPY 99-incus-agent.rules /usr/lib/udev/rules.d/99-incus-agent.rules
EOF'

# What the image holds, as key=value lines: the versions an Immich Mount check depends on.
inspect_image() {
  builder podman run --rm --entrypoint /bin/sh "$1" -c '
    . /usr/lib/os-release
    echo "version=$VERSION"
    echo "fedora=$(rpm -E %fedora)"
    for package in gtk4 libadwaita qt6-qtbase kf6-kirigami fuse3 glib2 gnome-shell nautilus plasma-workspace \
                   dolphin python3-gobject spectacle; do
      echo "$package=$(rpm -q --qf "%{VERSION}\\n" "$package" 2>/dev/null | head -1 | grep -v "not installed" || echo none)"
    done
    echo "homebrew=$(test -f /usr/share/homebrew.tar.zst && echo /usr/share/homebrew.tar.zst || echo none)"'
}

for edition in "${editions[@]}"; do
  upstream=$(upstream_image "$edition")
  local_image=localhost/bazzite-$edition-lab
  volume=$(base_volume "$edition")
  step "$edition: pulling $upstream in $BUILDER"
  builder sh -c "podman pull -q $upstream >/var/log/pull-$edition.log 2>&1" \
    || die "pull failed: $(builder tail -3 "/var/log/pull-$edition.log")"
  digest=$(builder podman image inspect --format '{{.Digest}}' "$upstream")
  step "$edition: building $local_image ($digest)"
  builder podman build -q --build-arg "BASE=$upstream" -t "$local_image" /root/lab-image >/dev/null
  facts=$(inspect_image "$local_image")
  printf '%s\n' "$facts" | sed 's/^/         /'

  # A run that failed half-way leaves the volume attached to a kept builder.
  inc config device remove --project "$PROJECT" "$BUILDER" target </dev/null >/dev/null 2>&1 || true
  if [ -n "$(inc storage volume list "$POOL" --project "$PROJECT" -f csv -c n "^$volume\$" </dev/null)" ]; then
    step "$edition: replacing $volume"
    inc storage volume delete "$POOL" "$volume" --project "$PROJECT" </dev/null
  fi
  inc storage volume create "$POOL" "$volume" --project "$PROJECT" --type=block size=50GiB </dev/null >/dev/null
  inc config device add --project "$PROJECT" "$BUILDER" target disk pool="$POOL" source="$volume" </dev/null >/dev/null
  wait_for "$PROJECT" "$BUILDER" "the $volume disk" test -b "$TARGET_DISK"
  # Bazzite's own bootc installs it, as `bootc install to-disk` documents: btrfs, as Bazzite's installer uses, and
  # the serial console, so `incus console --show-log` shows a boot that hangs. The builder runs with SELinux
  # disabled (Incus' Fedora image), so bootc cannot enter install_t; it labels the files from Bazzite's own policy
  # all the same, with the fallback it offers for that case.
  step "$edition: bootc install to-disk onto $volume"
  builder sh -c "podman run --rm --privileged --pid=host --security-opt label=type:unconfined_t \
      -e BOOTC_SETENFORCE0_FALLBACK=1 -v /dev:/dev -v /var/lib/containers:/var/lib/containers $local_image \
      bootc install to-disk --wipe --generic-image --filesystem btrfs --karg console=tty0 --karg console=ttyS0 \
      $TARGET_DISK >/var/log/install-$edition.log 2>&1" \
    || die "bootc install failed: $(builder tail -5 "/var/log/install-$edition.log")"
  inc config device remove --project "$PROJECT" "$BUILDER" target </dev/null >/dev/null

  settings=(user.keep=true "user.bazzite.upstream=$upstream" "user.bazzite.digest=$digest"
            "user.bazzite.built=$(date -u +%Y-%m-%dT%H:%MZ)")
  while IFS='=' read -r key value; do settings+=("user.bazzite.$key=$value"); done <<<"$facts"
  inc storage volume set "$POOL" "$volume" --project "$PROJECT" "${settings[@]}" </dev/null
  step "$edition: $volume ready"
done
