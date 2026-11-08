#!/usr/bin/env bash
# Immich in this machine's own Docker, seeded as the lab server is, for CI (the `contract` job in
# .github/workflows/ci.yml). The contract tests then run against http://127.0.0.1:2283 with the throwaway
# credentials this writes to .env.local. Machine learning stays off, as on the lab server without --ml.
#
#   lab/immich-server/ci.sh v3.3.6
#
# Needs Docker with compose, sudo, curl and jq: a GitHub-hosted Ubuntu runner has them. Not for a workstation:
# seed-container.sh installs packages and writes to /opt as root.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
VERSION=${1:?usage: $0 <immich version, such as v3.3.6>}
DIR=${IMMICH_DIR:-${RUNNER_TEMP:-/tmp}/immich}
step() { printf '%7ss  %s\n' "$SECONDS" "$*"; }
# wait_until SECONDS COMMAND...: poll every 2 s, fail after the deadline.
wait_until() {
  local deadline=$(( SECONDS + $1 )); shift
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || { echo "timed out waiting for: $*" >&2; return 1; }
    sleep 2
  done
}

step "Immich $VERSION in $DIR"
mkdir -p "$DIR" && cd "$DIR"
base=https://github.com/immich-app/immich/releases/download/$VERSION
curl -fsSL -o docker-compose.yml "$base/docker-compose.yml"
curl -fsSL -o example.env "$base/example.env"
sed -e "s/^IMMICH_VERSION=.*/IMMICH_VERSION=$VERSION/" \
    -e "s/^DB_PASSWORD=.*/DB_PASSWORD=$(head -c 300 /dev/urandom | tr -dc A-Za-z0-9 | cut -c1-24)/" \
    -e "s|^# TZ=Etc/UTC|TZ=Etc/UTC|" example.env > .env
cat > docker-compose.override.yml <<EOF
services:
  immich-machine-learning:
    profiles: [ml]
EOF
docker compose up -d --quiet-pull
wait_until 300 sh -c 'curl -fsS --max-time 3 http://127.0.0.1:2283/api/server/ping | grep -q pong'
step "Immich answers: $(curl -fsS http://127.0.0.1:2283/api/server/version)"

step "fixtures"
FIXTURES_ALL=1 FIXTURES_LENIENT=1 "$HERE/fixtures.sh"
sudo mkdir -p /opt/fixtures /opt/immich
sudo cp -r "$HERE/fixtures/manifest.json" "$HERE/fixtures/cache" /opt/fixtures/

sudo apt-get update -qq   # seed-container.sh installs jq, exiftool and ffmpeg
seed() { sudo bash "$HERE/seed-container.sh" "$@"; }
step "setup"; seed setup
step "upload"; seed upload
wait_until 900 seed idle
step "organize"; seed organize
wait_until 600 seed idle
{
  echo "# Throwaway credentials for the CI Immich server. Written by ci.sh."
  echo "IMMICH_URL=http://127.0.0.1:2283"
  sudo cat /opt/immich/seed/keys.env
} > "$HERE/.env.local"
step "$(seed summary)"
