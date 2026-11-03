#!/usr/bin/env bash
# Delete the disposable Immich container (and with it all uploads and the database).
#   ./down.sh      Env: IMMICH_INSTANCE (default ind-immich-srv), EREBUS_DIR
set -euo pipefail
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
NAME=${IMMICH_INSTANCE:-ind-immich-srv}
# shellcheck source=/dev/null
. "$EREBUS_DIR/lib/erebus.sh"
HERE=$(cd "$(dirname "$0")" && pwd)

if [ -n "$(state lab "$NAME")" ]; then
  inc delete --project lab -f "$NAME" </dev/null
  step "deleted $NAME"
else
  step "$NAME does not exist"
fi
# They point at the deleted server.
if [ "$NAME" = ind-immich-srv ]; then rm -f "$HERE/.env.local" "$HERE/.assets.tsv"; else rm -f "$HERE/.env.$NAME.local" "$HERE/.assets.$NAME.tsv"; fi
