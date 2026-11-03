#!/usr/bin/env bash
# The release check, from this machine: bring up (warm) the lab server and a desktop VM, install Immich Mount as its
# users would, sign in from the command line, and run desktop.sh in the VM's session. Prints one line per check and
# the wall time, and leaves the warm instances up for the next run (each kind's down.sh deletes them).
#
#   lab/check/run.sh <kind> [--release | --binary <path>] [--uninstall]
#
#   kind         gnome or plasma: a Fedora 44 VM, with the release's .rpm packages
#                bazzite-gnome or bazzite-kde: a Bazzite VM, with Homebrew's cask
#   --release    install the release the documented way for that system (the default)
#   --binary     the same, then a local build of immich-mount (the daemon and CLI) in place of the installed one
#   --uninstall  Bazzite only: then `brew uninstall --cask --zap`, and uninstall.sh. Without an install flag, only
#                that, on the VM as the last run left it
#
# Env: VERSION (the release to install and expect; default the latest on GitHub, from gh), IMMICH_INSTANCE, GNOME_VM,
#      PLASMA_VM, BAZZITE_VM and EREBUS_DIR, as the lab scripts take them.
#
# Signing in stores the credential in a file (the setting credential.file), not the keyring, so no keyring prompt
# waits for an answer. The KDE kinds then run the line `immich-mount doctor` prints to turn Dolphin's previews on,
# which the KDE app's first run does with its Show thumbnails switch.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
lab=$(dirname "$here")
EREBUS_DIR=${EREBUS_DIR:-$(cd "$here/../../.." && pwd)/erebus}
[ -f "$EREBUS_DIR/lib/erebus.sh" ] || { echo "no lib/erebus.sh in EREBUS_DIR=$EREBUS_DIR" >&2; exit 1; }
export EREBUS_DIR
. "$EREBUS_DIR/lib/erebus.sh"
project=lab

usage="usage: $0 gnome|plasma|bazzite-gnome|bazzite-kde [--release | --binary <path>] [--uninstall]"
kind=${1:-}
[ $# -gt 0 ] && shift
install=
binary=
uninstall=false
while [ $# -gt 0 ]; do
  case $1 in
    --release) install=release ;;
    --binary) install=binary; binary=$(realpath "${2:?$usage}"); shift ;;
    --uninstall) uninstall=true ;;
    *) die "$usage" ;;
  esac
  shift
done
[ -n "$install" ] || $uninstall || install=release

case $kind in
  gnome) vm_dir=$lab/gnome-vm; VM=${GNOME_VM:-ind-gnome-1}; export GNOME_VM=$VM; up_arguments=(); channel=rpm; interface=gnome ;;
  plasma) vm_dir=$lab/plasma-vm; VM=${PLASMA_VM:-ind-plasma-1}; export PLASMA_VM=$VM; up_arguments=(); channel=rpm; interface=kde ;;
  bazzite-gnome) vm_dir=$lab/bazzite-vm; VM=${BAZZITE_VM:-ind-bazzite-gnome-1}; export BAZZITE_VM=$VM; up_arguments=(gnome); channel=brew; interface=gnome ;;
  bazzite-kde) vm_dir=$lab/bazzite-vm; VM=${BAZZITE_VM:-ind-bazzite-kde-1}; export BAZZITE_VM=$VM; up_arguments=(kde); channel=brew; interface=kde ;;
  *) die "$usage" ;;
esac
$uninstall && [ "$channel" != brew ] && die "--uninstall is for the Bazzite kinds: it checks Homebrew's cask uninstall"

server=${IMMICH_INSTANCE:-ind-immich-srv}
server_env=$lab/immich-server/.env.local
[ "$server" = ind-immich-srv ] || server_env=$lab/immich-server/.env.$server.local
# Lab VMs on the NAT network reach the server by its instance name.
server_url=http://$server.incus:2283
check_directory=/var/tmp/immich-check
logs=$(mktemp -d)
trap 'rm -rf "$logs"' EXIT

in_vm() { inc exec --project "$project" "$VM" -- "$@" </dev/null; }
in_session() { "$vm_dir/run-in-session.sh" "$@"; }
# A shell as a terminal in the session has it: Homebrew's programs at the end of the PATH, as on Bazzite (a path that
# does not exist changes nothing elsewhere), and the desktop's variables from the user manager, where the session
# put them (immich-mount doctor reads XDG_CURRENT_DESKTOP).
session_shell() {
  local script=$1
  shift
  in_session bash -c "PATH=\$PATH:/home/linuxbrew/.linuxbrew/bin
    eval \"\$(systemctl --user show-environment | grep -E '^(XDG_CURRENT_DESKTOP|XDG_SESSION_TYPE|KDE_FULL_SESSION|KDE_SESSION_VERSION)=' | sed 's/^/export /')\"
    $script" _ "$@"
}
# logged <name> <command...>: run quietly; on failure show the end of its output and stop.
logged() {
  local name=$1
  shift
  "$@" >"$logs/$name" 2>&1 || { tail -15 "$logs/$name" >&2; die "$name failed"; }
}

# The check's scripts and what desktop.sh asks the server with, pushed on every run: a warm VM starts without them.
push_check_files() {
  local uid
  uid=$(in_vm id -u tester)
  in_vm sh -c "rm -rf $check_directory && mkdir -p $check_directory && chown $uid:$uid $check_directory"
  inc file push --project "$project" --uid "$uid" --gid "$uid" --mode 0755 "$here/desktop.sh" "$here/uninstall.sh" \
    "$VM$check_directory/" </dev/null
  . "$server_env"
  printf 'IMMICH_URL=%s\nIMMICH_API_KEY_READONLY=%s\n' "$server_url" "$IMMICH_API_KEY_READONLY" >"$logs/server.env"
  inc file push --project "$project" --uid "$uid" --gid "$uid" --mode 0600 "$logs/server.env" "$VM$check_directory/server.env" </dev/null
}

install_release() {
  case $channel in
    brew)
      # WORKFLOWS.md, "Install": the one command, in a terminal.
      logged install in_session bash -ic 'brew install jakobhviid/tap/immich-mount' ;;
    rpm)
      # WORKFLOWS.md, "Install", Fedora 44: the daemon's package and the interface for the desktop, from the release.
      local base=https://github.com/jakobhviid/immich-mount/releases/download/v$version
      logged install in_vm sh -c 'set -e; base=$1; interface=$2; shift 2
        rm -rf /root/release; mkdir /root/release; cd /root/release
        for name in "$@"; do curl -fsSLO "$base/$name"; done
        dnf -y install ./immich-mount-[0-9]*.x86_64.rpm ./immich-mount-"$interface"-*.rpm' _ \
        "$base" "$interface" "immich-mount-$version-1.x86_64.rpm" "immich-mount-$interface-$version-1.fc44.x86_64.rpm" ;;
  esac
}

# The local build in place of the installed program, so the launcher, the icons and the unit stay the release's.
install_binary() {
  local installed owner
  installed=$(session_shell 'readlink -f "$(command -v immich-mount)"')
  [ -n "$installed" ] || die "no installed immich-mount to replace"
  owner=$(in_vm stat -c %u "$installed")
  inc file push --project "$project" --uid "$owner" --gid "$owner" --mode 0755 "$binary" "$VM$installed" </dev/null
  version=$(session_shell 'immich-mount --version' | awk '{print $2}')
}

sign_in() {
  . "$server_env"
  logged sign-in session_shell 'set -e
    immich-mount configure set credential.file "$HOME/.config/immich-mount/credential"
    printf "%s\n%s\n" "$1" "$2" | immich-mount login --server "$3" --password
    immich-mount mount' "$IMMICH_ADMIN_EMAIL" "$IMMICH_ADMIN_PASSWORD" "$server_url"
  if [ "$interface" = kde ]; then
    local fix
    fix=$(session_shell 'immich-mount doctor --json' 2>/dev/null |
      jq -r '.checks[] | select(.name == "dolphin-previews" and .status == "warning") | .fix // empty')
    [ -z "$fix" ] || logged previews session_shell "$fix"
  fi
}

if [ -n "$install" ]; then
  if [ "$install" = release ] || [ -z "${VERSION:-}" ]; then
    version=${VERSION:-$(gh release view -R jakobhviid/immich-mount --json tagName -q .tagName)}
    version=${version#v}
  fi
  step "bringing up $server and $VM"
  run_bg "$logs/server-up" bash -c '"$1/up.sh" && "$1/seed.sh"' _ "$lab/immich-server"
  run_bg "$logs/vm-up" "$vm_dir/up.sh" "${up_arguments[@]}"
  wait
  for name in server-up vm-up; do
    [ "$(cat "$logs/$name.rc")" = 0 ] || { tail -15 "$logs/$name" >&2; die "$name failed"; }
  done
  logged session "$vm_dir/session.sh"
  step "installing $version ($channel)"
  install_release
  [ "$install" = binary ] && install_binary && step "replaced by $binary ($version)"
  step "signing in and mounting"
  sign_in
  push_check_files
  step "checking"
  set +e
  in_session "$check_directory/desktop.sh" "$version" | tee "$logs/table"
  set -e
  passed=$(grep -c '^PASS' "$logs/table" || true)
  failed=$(grep -c '^FAIL' "$logs/table" || true)
  skipped=$(grep -c '^SKIP' "$logs/table" || true)
  step "$kind, $version: $passed passed, $failed failed, $skipped skipped"
fi

if $uninstall; then
  [ -n "$install" ] || push_check_files
  step "brew uninstall --cask --zap jakobhviid/tap/immich-mount"
  logged uninstall in_session bash -ic 'brew uninstall --cask --zap jakobhviid/tap/immich-mount'
  set +e
  in_session "$check_directory/uninstall.sh" | tee "$logs/uninstall-table"
  set -e
  failed=$(( ${failed:-0} + $(grep -c '^FAIL' "$logs/uninstall-table" || true) ))
  step "$kind uninstalled: $(grep -c '^PASS' "$logs/uninstall-table" || true) passed, $(grep -c '^FAIL' "$logs/uninstall-table" || true) failed"
fi
step "wall time"
[ "${failed:-0}" = 0 ]
