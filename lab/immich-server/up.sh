#!/usr/bin/env bash
# Create a disposable Immich server on Erebus (project `lab`).
#
#   ./up.sh            NAT network (incusbr0): reachable from Erebus and its other instances only
#   ./up.sh --lan      --profile default --profile lan: a real LAN address, reachable from this host
#   ./up.sh --fresh    build it again, even when it is warm
#   ./up.sh --ml       with machine learning (smart search, faces, OCR, duplicates), as ind-immich-ml unless
#                      IMMICH_INSTANCE names another; slower to seed (models download, CPU inference)
#
# Warm in NAT mode: once seed.sh has run, it takes a snapshot of the seeded server, and later starts restore it (the
# credentials in .env.local stay valid). A server other than ind-immich-srv writes .env.<name>.local instead.
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

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/../warm.sh"
PROFILES=()
build=false
ML=false
for argument in "$@"; do
  case $argument in
    --lan) PROFILES=(--profile default --profile lan) ;;
    --fresh) build=true ;;
    --ml) ML=true; [ -n "${IMMICH_INSTANCE:-}" ] || NAME=ind-immich-ml ;;
    *) die "usage: $0 [--lan] [--fresh] [--ml]" ;;
  esac
done
# Each instance its own credentials, so two servers can run side by side.
ENV_FILE=$HERE/.env.local ASSETS_FILE=$HERE/.assets.tsv
[ "$NAME" = ind-immich-srv ] || { ENV_FILE=$HERE/.env.$NAME.local ASSETS_FILE=$HERE/.assets.$NAME.tsv; }
sweep_expired
# Warm, in NAT mode only (a LAN-mode server shows its throwaway login to the whole LAN): back to the snapshot seed.sh
# took, seeded, with the credentials it wrote.
if [ ${#PROFILES[@]} -eq 0 ] && ! $build && has_fresh "$NAME" && [ -f "$ENV_FILE" ]; then
  step "restoring $NAME to its seeded snapshot"
  restore_fresh "$NAME"
  wait_for "$P" "$NAME" "Immich /api/server/ping" \
    sh -c 'curl -fsS --max-time 3 http://127.0.0.1:2283/api/server/ping | grep -q pong'
  IP=$(inc list --project "$P" "^$NAME\$" -f json </dev/null |
       jq -r '.[0].state.network.eth0.addresses[] | select(.family=="inet") | .address' | head -1)
  sed -i "s|^IMMICH_URL=.*|IMMICH_URL=http://$IP:2283|" "$ENV_FILE"
  step "ready: $NAME, seeded as before (seed.sh has nothing to do)"
  exit 0
fi
if [ -n "$(state "$P" "$NAME")" ]; then
  step "deleting $NAME to build it again"
  inc delete --project "$P" -f "$NAME" </dev/null
  rm -f "$ENV_FILE" "$ASSETS_FILE"
fi

step "launch $NAME ($IMAGE, ${PROFILES[*]:-NAT}, Immich $IMMICH_VERSION)"
# Not --ephemeral: Erebus reboots itself for OS updates, and an ephemeral instance
# would be deleted by that. Not user.keep either: teardown may remove it. ./down.sh deletes it when the session
# ends, and user.expires lets a later session delete it if that was forgotten.
inc launch "$IMAGE" "$NAME" --project "$P" "${PROFILES[@]}" -c user.expires="$(expires_at)" \
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

step "fetch docker-compose.yml and example.env of $IMMICH_VERSION (machine learning: $ML)"
inc exec --project "$P" "$NAME" --env V="$IMMICH_VERSION" --env ML="$ML" -- bash -euo pipefail -c '
  mkdir -p /opt/immich && cd /opt/immich
  base=https://github.com/immich-app/immich/releases/download/$V
  curl -fsSL -o docker-compose.yml "$base/docker-compose.yml"
  curl -fsSL -o example.env "$base/example.env"
  # .env: pin the exact version (example.env says only "v3"), a random DB password, UTC.
  sed -e "s/^IMMICH_VERSION=.*/IMMICH_VERSION=$V/" \
      -e "s/^DB_PASSWORD=.*/DB_PASSWORD=$(tr -dc A-Za-z0-9 </dev/urandom | head -c 24)/" \
      -e "s|^# TZ=Etc/UTC|TZ=Etc/UTC|" example.env > .env
  # Without --ml, drop machine learning without editing the upstream file: an override puts the
  # service behind a profile that is never activated. (seed.sh also disables ML
  # in the server config, so jobs do not try to reach it.) With --ml, a marker tells seed-container.sh to keep it.
  if [ "$ML" = true ]; then
    touch ml-enabled
  else
    cat > docker-compose.override.yml <<EOF
services:
  immich-machine-learning:
    profiles: [ml]
EOF
  fi
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
