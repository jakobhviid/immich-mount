#!/bin/sh
# In a throwaway instance, as root: install the release packages in /root/packages that fit this distribution, check
# them, and remove them again. The last line is the verdict; the exit code is the result.
#
#   The daemon and CLI package everywhere; the GNOME and KDE apps on Fedora 44 (rpm) and Ubuntu 26.04 (deb).
#   With /root/server.env (IMMICH_URL and IMMICH_KEY, a read-only API key), it also signs in, mounts as root, and
#   checks that originals read through the mount match Immich's own checksums.
set -u
. /etc/os-release
cd /root/packages || { echo "FAIL: no /root/packages"; exit 1; }
fail() { echo "FAIL: $*"; exit 1; }
apps=
if command -v dnf >/dev/null; then
  packages=$(ls immich-mount-[0-9]*."$(uname -m)".rpm)
  if [ "$ID" = fedora ] && [ "$VERSION_ID" = 44 ]; then
    packages="$packages $(ls immich-mount-gnome-*.fc44.*.rpm immich-mount-kde-*.fc44.*.rpm)"; apps=yes
  fi
  dnf -y -q install $packages >/tmp/install.log 2>&1 || { tail -5 /tmp/install.log; fail "dnf install"; }
  remove() { dnf -y -q remove immich-mount >/tmp/remove.log 2>&1; }
elif command -v apt-get >/dev/null; then
  arch=$(dpkg --print-architecture)
  packages=$(ls ./immich-mount_*_"$arch".deb)
  if [ "$ID" = ubuntu ] && [ "$VERSION_ID" = 26.04 ]; then
    packages="$packages $(ls ./immich-mount-gnome_*_"$arch".deb ./immich-mount-kde_*_"$arch".deb)"; apps=yes
  fi
  export DEBIAN_FRONTEND=noninteractive
  apt-get -qq update >/dev/null
  apt-get -y -qq install $packages >/tmp/install.log 2>&1 || { tail -5 /tmp/install.log; fail "apt install"; }
  remove() { apt-get -y -qq purge immich-mount >/tmp/remove.log 2>&1; }
else
  fail "no dnf or apt-get"
fi

expected=$(ls immich-mount-[0-9]*.rpm immich-mount_*.deb 2>/dev/null | head -1 | sed -E 's/^\.?\/?immich-mount[-_]([0-9]+\.[0-9]+\.[0-9]+).*/\1/')
version=$(immich-mount --version | awk '{print $2}')
[ "$version" = "$expected" ] || fail "--version says $version, the package $expected"
[ -f /usr/lib/systemd/user/immich-mount.service ] || fail "no user unit"
[ -f /usr/share/icons/hicolor/scalable/apps/dev.hviid.ImmichMount.svg ] || fail "no icon"
immich-mount doctor --json 2>/dev/null | head -c 1 | grep -q '{' || fail "doctor --json printed no JSON"
if [ -n "$apps" ]; then
  for app in immich-mount-gnome immich-mount-kde; do
    [ -x "/usr/bin/$app" ] || fail "no /usr/bin/$app"
    missing=$(ldd "/usr/bin/$app" | grep -c "not found")
    [ "$missing" = 0 ] || fail "$app misses $missing libraries"
  done
  grep -q "x-scheme-handler/app.immich" /usr/share/applications/mimeinfo.cache || fail "app.immich not registered"
  # Each app starts: GTK under a headless Wayland compositor, Qt off screen. Still running after 8 seconds means
  # its interface and libraries loaded.
  if command -v dnf >/dev/null; then dnf -y -q install weston; else apt-get -y -qq install weston; fi >/dev/null 2>&1
  mkdir -p /tmp/runtime && chmod 700 /tmp/runtime
  XDG_RUNTIME_DIR=/tmp/runtime weston --backend=headless --socket=wayland-test >/tmp/weston.log 2>&1 &
  weston=$!
  sleep 3
  XDG_RUNTIME_DIR=/tmp/runtime WAYLAND_DISPLAY=wayland-test timeout 8 immich-mount-gnome >/tmp/gnome.log 2>&1
  [ $? = 124 ] || fail "immich-mount-gnome stopped at once: $(tail -1 /tmp/gnome.log)"
  XDG_RUNTIME_DIR=/tmp/runtime QT_QPA_PLATFORM=offscreen timeout 8 immich-mount-kde >/tmp/kde.log 2>&1
  [ $? = 124 ] || fail "immich-mount-kde stopped at once: $(tail -1 /tmp/kde.log)"
  kill "$weston" 2>/dev/null
fi
mounted=
if [ -f /root/server.env ]; then
  . /root/server.env
  export HOME=/root
  command -v curl >/dev/null || { if command -v dnf >/dev/null; then dnf -y -q install curl; else apt-get -y -qq install curl; fi; } >/dev/null 2>&1
  printf '%s' "$IMMICH_KEY" | immich-mount login --server "$IMMICH_URL" --api-key >/tmp/login.log 2>&1 || fail "login: $(tail -1 /tmp/login.log)"
  mkdir -p /root/Immich
  immich-mount configure set mount.path /root/Immich >/dev/null || fail "configure"
  immich-mount daemon >/tmp/daemon.log 2>&1 &
  daemon=$!
  for n in $(seq 30); do grep -q " /root/Immich fuse" /proc/mounts && break; sleep 1; done
  grep -q " /root/Immich fuse" /proc/mounts || fail "not mounted: $(tail -2 /tmp/daemon.log)"
  # Immich's SHA-1 of every original, from base64 to hex.
  curl -s -m 30 -H "x-api-key: $IMMICH_KEY" -H "content-type: application/json" -d '{"size":1000}' "$IMMICH_URL/api/search/metadata" \
    | grep -o '"checksum":"[^"]*"' | cut -d'"' -f4 | while read -r checksum; do printf '%s' "$checksum" | base64 -d | od -An -tx1 | tr -d ' \n'; echo; done >/tmp/checksums
  [ -s /tmp/checksums ] || fail "no checksums from the server"
  find /root/Immich/Photos -type f | head -5 >/tmp/files
  checked=0
  while IFS= read -r file; do
    grep -qx "$(sha1sum "$file" | cut -d' ' -f1)" /tmp/checksums || fail "$file does not match any original on the server"
    checked=$((checked + 1))
  done </tmp/files
  [ "$checked" -gt 0 ] || fail "no file under Photos"
  immich-mount unmount >/dev/null 2>&1 || fail "unmount"
  # The daemon only: a bare `wait` would also wait for anything else still running.
  wait "$daemon"
  mounted=", mounted with $checked originals matching Immich's checksums"
fi
remove || { tail -5 /tmp/remove.log; fail "remove"; }
[ ! -e /usr/bin/immich-mount ] && [ ! -e /usr/bin/immich-mount-gnome ] && [ ! -e /usr/bin/immich-mount-kde ] || fail "files left after removal"
echo "ok: $version, daemon${apps:+ and both apps (started)}, installed${mounted}, checked and removed"
