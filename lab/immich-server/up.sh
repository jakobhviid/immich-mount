#!/usr/bin/env bash
# Create a disposable Immich server on Erebus (project `lab`).
#
#   ./up.sh            NAT network (incusbr0): reachable from Erebus and its other instances only
#   ./up.sh --lan      --profile default --profile lan: a real LAN address, reachable from this host
#
# Env overrides:
#   IMMICH_VERSION   Immich release tag to deploy            (default: v3.3.5, the version this was verified with)
#   IMMICH_INSTANCE  Incus instance name                     (default: ind-immich-srv)
#   IMMICH_IMAGE     Incus image                             (default: images:debian/13)
#   EREBUS_DIR       path of the erebus repo                 (default: erebus next to this repository)
#
# Afterwards: ./seed.sh (admin, API keys, fixtures), ./down.sh (delete it).
set -euo pipefail

EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
IMMICH_VERSION=${IMMICH_VERSION:-v3.3.5}
NAME=${IMMICH_INSTANCE:-ind-immich-srv}
IMAGE=${IMMICH_IMAGE:-images:debian/13}
P=lab
# shellcheck source=/dev/null
. "$EREBUS_DIR/lib/erebus.sh"   # inc, step, die, wait_for, state

PROFILES=()
case ${1:-} in
  --lan) PROFILES=(--profile default --profile lan) ;;
  "") ;;
  *) die "usage: $0 [--lan]" ;;
esac

[ -z "$(state "$P" "$NAME")" ] || die "$NAME already exists; run ./down.sh first"

step "launch $NAME ($IMAGE, ${PROFILES[*]:-NAT}, Immich $IMMICH_VERSION)"
# Not --ephemeral: Erebus reboots itself for OS updates, and an ephemeral instance
# would be deleted by that. Not user.keep either: teardown may remove it. ./down.sh deletes it.
inc launch "$IMAGE" "$NAME" --project "$P" "${PROFILES[@]}" \
  -c security.nesting=true -c limits.cpu=4 -c limits.memory=8GiB -d root,size=40GiB </dev/null

# In --lan mode the address comes from the router's DHCP; DNS working implies it arrived.
wait_for "$P" "$NAME" "network" getent hosts download.docker.com

step "install Docker Engine (docker.com apt repo) and compose plugin"
inc exec --project "$P" "$NAME" -- bash -euo pipefail -c '
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq ca-certificates curl jq >/dev/null
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
  . /etc/os-release
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian $VERSION_CODENAME stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -qq
  apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin >/dev/null
' </dev/null
wait_for "$P" "$NAME" "docker daemon" docker info
inc exec --project "$P" "$NAME" -- docker info --format 'docker {{.ServerVersion}}, storage driver {{.Driver}}' </dev/null

step "fetch docker-compose.yml and example.env of $IMMICH_VERSION"
inc exec --project "$P" "$NAME" --env V="$IMMICH_VERSION" -- bash -euo pipefail -c '
  mkdir -p /opt/immich && cd /opt/immich
  base=https://github.com/immich-app/immich/releases/download/$V
  curl -fsSL -o docker-compose.yml "$base/docker-compose.yml"
  curl -fsSL -o example.env "$base/example.env"
  # .env: pin the exact version (example.env says only "v3"), a random DB password, UTC.
  sed -e "s/^IMMICH_VERSION=.*/IMMICH_VERSION=$V/" \
      -e "s/^DB_PASSWORD=.*/DB_PASSWORD=$(tr -dc A-Za-z0-9 </dev/urandom | head -c 24)/" \
      -e "s|^# TZ=Etc/UTC|TZ=Etc/UTC|" example.env > .env
  # Drop machine learning without editing the upstream file: an override puts the
  # service behind a profile that is never activated. (seed.sh also disables ML
  # in the server config, so jobs do not try to reach it.)
  cat > docker-compose.override.yml <<EOF
services:
  immich-machine-learning:
    profiles: [ml]
EOF
  docker compose config --services | sort | tr "\n" " "; echo
' </dev/null

step "pull images"
inc exec --project "$P" "$NAME" --cwd /opt/immich -- docker compose pull -q </dev/null
step "start stack"
inc exec --project "$P" "$NAME" --cwd /opt/immich -- docker compose up -d --quiet-pull </dev/null
wait_for "$P" "$NAME" "Immich /api/server/ping" \
  sh -c 'curl -fsS --max-time 3 http://127.0.0.1:2283/api/server/ping | grep -q pong'
step "Immich answers /api/server/ping"

IP=$(inc list --project "$P" "^$NAME\$" -f json </dev/null |
     jq -r '.[0].state.network.eth0.addresses[] | select(.family=="inet") | .address' | head -1)
URL="http://$IP:2283"
inc exec --project "$P" "$NAME" -- curl -fsS http://127.0.0.1:2283/api/server/version </dev/null; echo

# Can this machine reach it? Expected yes with --lan, no on NAT (incusbr0 is not routed to the LAN).
if curl -fsS --max-time 5 "$URL/api/server/ping" >/dev/null 2>&1; then
  step "reachable from $(hostname): $URL"
else
  step "NOT reachable from $(hostname): $URL"
  cat <<EOF
  From another Erebus instance on the NAT network: http://$NAME.incus:2283 (or $URL)
  From this host, a tunnel through incus exec (needs socat on both sides):
    $EREBUS_DIR/bin/incus exec $NAME -- apt-get install -y socat
    socat TCP-LISTEN:2283,bind=127.0.0.1,reuseaddr,fork \\
      EXEC:"$EREBUS_DIR/bin/incus exec $NAME -- socat STDIO TCP\:127.0.0.1\:2283"
    # then http://127.0.0.1:2283
EOF
fi
