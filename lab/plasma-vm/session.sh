#!/usr/bin/env bash
# Start a headless Plasma session for tester: KWin's virtual backend with one output, and plasmashell in it, on
# tester's lingering systemd --user manager. Every Qt app has accessibility on (act.py drives them through it), and
# KWin takes screenshots without asking (shot.sh). Idempotent: restarts the session.
#
#   lab/plasma-vm/session.sh [WIDTHxHEIGHT]      (default 1440x900)
#   logs:  lab/plasma-vm/run-in-session.sh journalctl --user -u plasma-headless
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
size=${1:-1440x900}
uid=$(vm id -u "$TESTER")
is() { vm /usr/local/bin/in-session "$@"; }
# What a Plasma login provides, for the session and for every app D-Bus or systemd starts later (Dolphin, the KIO
# workers, ksecretd, the portals), which take their environment from the user manager.
environment=(XDG_CURRENT_DESKTOP=KDE KDE_FULL_SESSION=true KDE_SESSION_VERSION=6 XDG_SESSION_TYPE=wayland
             XDG_SESSION_CLASS=user WAYLAND_DISPLAY=wayland-0 QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
             KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1)
step "session settings (no screen locking)"
is kwriteconfig6 --file kscreenlockerrc --group Daemon --key Autolock false
is systemctl --user set-environment "${environment[@]}"
is systemctl --user stop plasma-headless.service 2>/dev/null || true
is systemctl --user reset-failed plasma-headless.service 2>/dev/null || true
unit_environment=()
for variable in "${environment[@]}"; do unit_environment+=(-E "$variable"); done
step "starting kwin_wayland --virtual $size, with plasmashell"
is systemd-run --user --unit=plasma-headless --collect "${unit_environment[@]}" \
  kwin_wayland --virtual --width "${size%x*}" --height "${size#*x}" --no-lockscreen --xwayland \
  --exit-with-session plasmashell
wait_for "$P" "$VM" "the Wayland socket" test -S "/run/user/$uid/wayland-0"
wait_for "$P" "$VM" "plasmashell on the session bus" /usr/local/bin/in-session \
  busctl --user status org.kde.plasmashell
step "session up: $(is plasmashell --version 2>/dev/null)"
