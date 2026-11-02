#!/usr/bin/env bash
# Start a headless GNOME Shell for tester, with one virtual monitor, on tester's
# lingering systemd --user manager. Its session bus (/run/user/<uid>/bus) is the
# one run-in-session.sh and shot.sh talk to. Idempotent: restarts the shell.
#
#   lab/gnome-vm/session.sh [WIDTHxHEIGHT]      (default 1280x800)
#   logs:  lab/gnome-vm/run-in-session.sh journalctl --user -u gnome-headless
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"
size=${1:-1280x800}
uid=$(vm id -u "$TESTER")
is() { vm /usr/local/bin/in-session "$@"; }

step "session settings (no lock, no blanking, no animations, no welcome dialog)"
is gsettings set org.gnome.desktop.session idle-delay 0
is gsettings set org.gnome.desktop.screensaver lock-enabled false
is gsettings set org.gnome.desktop.interface enable-animations false
# A new virtual pointer (exp/input.sh) can trip the top-left hot corner.
is gsettings set org.gnome.desktop.interface enable-hot-corners false
# Otherwise a "Welcome to Fedora Linux 44 / Take Tour" dialog covers the screen.
is gsettings set org.gnome.shell welcome-dialog-last-shown-version "'9999'"

# A transient unit, so the shell lives in the user manager rather than in this
# `incus exec`, and its log lands in tester's journal.
is systemctl --user stop gnome-headless.service 2>/dev/null || true
is systemctl --user reset-failed gnome-headless.service 2>/dev/null || true
step "starting gnome-shell --headless --unsafe-mode --virtual-monitor $size"
is systemd-run --user --unit=gnome-headless --collect -E XDG_SESSION_TYPE=wayland \
  gnome-shell --headless --unsafe-mode --virtual-monitor "$size"

wait_for "$P" "$VM" "the Wayland socket" test -S "/run/user/$uid/wayland-0"
wait_for "$P" "$VM" "org.gnome.Shell on the session bus" /usr/local/bin/in-session \
  gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
  --method org.freedesktop.DBus.Properties.Get org.gnome.Shell ShellVersion

# D-Bus/systemd-activated apps (Nautilus, portals, thumbnailers) take their
# environment from the user manager, not from us: hand them the display.
# XDG_SESSION_CLASS=user is what a real logind login would provide; without it
# localsearch-3.service is skipped (ConditionEnvironment=XDG_SESSION_CLASS=user).
is systemctl --user set-environment WAYLAND_DISPLAY=wayland-0 XDG_SESSION_TYPE=wayland \
  XDG_SESSION_CLASS=user XDG_CURRENT_DESKTOP=GNOME

# GNOME starts in the overview; leave it so windows are visible. Eval is only
# allowed with --unsafe-mode.
out=$(is gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
  --method org.gnome.Shell.Eval 'Main.overview.hide(); true')
[ "$out" = "(true, 'true')" ] || die "Eval failed (is --unsafe-mode on?): $out"

step "session up: $(is gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell \
  --method org.freedesktop.DBus.Properties.Get org.gnome.Shell ShellVersion)"
