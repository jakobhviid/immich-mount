#!/usr/bin/env bash
# Install a release's packages on several distributions at once, each in a throwaway container on Erebus, and print
# one row per distribution (install-test.sh decides what fits where and what is checked).
#
#   lab/packaging/matrix.sh [TAG] [DISTRO ...]     (default: the latest release; the distributions below)
#
# Instances are named ind-pkg-<distribution>, in the lab project, ephemeral, and stopped (so deleted) at the end.
# With the lab server up (lab/immich-server/.env.local), each also mounts it with its read-only key and checks
# originals against Immich's checksums.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
. "$EREBUS_DIR/lib/erebus.sh"
P=lab
tag=${1:-$(gh release view -R jakobhviid/immich-mount --json tagName -q .tagName </dev/null)}
shift || true
distros=("$@")
[ ${#distros[@]} -gt 0 ] || distros=(fedora/44 fedora/43 ubuntu/26.04 ubuntu/24.04 debian/13)

packages=$(mktemp -d /tmp/immich-mount-packages.XXXX)
server_env=
# The downloads and the key go; the logs in $out stay for reading.
trap 'rm -rf "$packages" ${server_env:+"$server_env"}' EXIT
if [ -f "$here/../immich-server/.env.local" ]; then
  server_env=$(mktemp /tmp/immich-mount-server.XXXX)
  ( . "$here/../immich-server/.env.local"; printf 'IMMICH_URL=%s\nIMMICH_KEY=%s\n' "$IMMICH_URL" "$IMMICH_API_KEY_READONLY" ) >"$server_env"
fi
out=$(mktemp -d /tmp/immich-mount-matrix.XXXX)
step "downloading $tag"
# gh can hang after the last asset; the files are complete by then, so a timeout is safe.
(cd "$packages" && timeout 120 gh release download "$tag" -R jakobhviid/immich-mount --pattern '*.rpm' --pattern '*.deb' </dev/null) || true
ls "$packages"/*.rpm "$packages"/*.deb >/dev/null 2>&1 || die "no packages downloaded for $tag"

# Runs in the background, so errexit is off here: a failing step must still stop the instance and leave a row.
run_one() {
  set +e
  local distro=$1 name log verdict
  name="ind-pkg-$(tr '/.' '--' <<<"$distro")"; log="$out/$name.log"
  {
    if inc launch "images:$distro" "$name" --project "$P" --ephemeral -q </dev/null; then
      if wait_for "$P" "$name" "network" sh -c 'getent hosts github.com >/dev/null' >/dev/null 2>&1; then
        for package in "$packages"/*; do
          inc file push --project "$P" --create-dirs "$package" "$name/root/packages/$(basename "$package")" </dev/null
        done
        inc file push --project "$P" "$here/install-test.sh" "$name/root/install-test.sh" --mode 0755 </dev/null
        [ -z "$server_env" ] || inc file push --project "$P" "$server_env" "$name/root/server.env" --mode 0600 </dev/null
        inc exec --project "$P" "$name" -- /root/install-test.sh </dev/null
      else
        echo "FAIL: no network"
      fi
      inc stop --project "$P" "$name" -q </dev/null
    else
      echo "FAIL: launch"
    fi
  } >"$log" 2>&1
  verdict=$(tail -1 "$log")
  case $verdict in
    ok:*) ;;
    FAIL:*) verdict="$verdict (log: $log)" ;;
    *) verdict="FAIL: ${verdict:-no verdict} (log: $log)" ;;
  esac
  printf '%-14s %s\n' "$distro" "$verdict" >"$out/$name.row"
}
step "testing on ${distros[*]}"
for distro in "${distros[@]}"; do run_one "$distro" & done
wait
cat "$out"/*.row
rows=$(ls "$out"/*.row 2>/dev/null | wc -l)
[ "$rows" -eq ${#distros[@]} ] || die "$rows rows for ${#distros[@]} distributions"
! grep -q "FAIL" "$out"/*.row
