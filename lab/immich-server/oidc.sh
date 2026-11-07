#!/usr/bin/env bash
# Give the lab server an OIDC provider: Dex, with one static user, and Immich's OAuth settings pointed at it.
# Run after ./up.sh and ./seed.sh. Appends the Dex user's throwaway password to .env.local.
#
#   ./oidc.sh
#
# Dex listens on port 5556 of the same instance, with the issuer http://<instance address>:5556/dex. Its client
# allows Immich's mobile redirect (app.immich:///oauth-callback), the address `immich-mount login` uses. The user
# is admin@example.com, so the OIDC sign-in lands on the seeded admin account.
#
# The issuer is plain http, so the settings include `allowInsecureRequests` (Immich refuses http issuers otherwise).
# The browser half of a sign-in is ./oidc-sign-in.py, which drives `immich-mount login` end to end.
# Env overrides: IMMICH_INSTANCE (default ind-immich-srv), EREBUS_DIR, DEX_IMAGE.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
EREBUS_DIR=${EREBUS_DIR:-$(cd "$(dirname "$0")/../../.." && pwd)/erebus}
NAME=${IMMICH_INSTANCE:-ind-immich-srv}
DEX_IMAGE=${DEX_IMAGE:-ghcr.io/dexidp/dex:v2.44.0}
P=lab
# shellcheck source=/dev/null
. "$EREBUS_DIR/lib/erebus.sh"
[ "$(state "$P" "$NAME")" = RUNNING ] || die "$NAME is not running; run ./up.sh first"
[ -f "$HERE/.env.local" ] || die "no .env.local; run ./seed.sh first"
# shellcheck source=/dev/null
. "$HERE/.env.local"
in_ct() { inc exec --project "$P" "$NAME" -- "$@" </dev/null; }

HOST=$(printf '%s' "$IMMICH_URL" | sed -E 's#^https?://([^:/]+).*#\1#')
ISSUER="http://$HOST:5556/dex"
CLIENT_SECRET=$(head -c 24 /dev/urandom | base64 | tr -d '/+=')
DEX_PASSWORD=$(head -c 18 /dev/urandom | base64 | tr -d '/+=')

step "dex: bcrypt the throwaway password"
HASH=$(in_ct docker run --rm httpd:2.4-alpine htpasswd -bnBC 10 "" "$DEX_PASSWORD" | tr -d ':\n')
[ -n "$HASH" ] || die "could not hash the Dex password"

step "dex: configure and start $DEX_IMAGE at $ISSUER"
in_ct mkdir -p /opt/dex
inc exec --project "$P" "$NAME" -- sh -c 'cat > /opt/dex/config.yaml' <<EOF
issuer: $ISSUER
storage:
  type: memory
web:
  http: 0.0.0.0:5556
oauth2:
  skipApprovalScreen: true
  passwordConnector: local
enablePasswordDB: true
staticClients:
  - id: immich
    name: Immich
    secret: $CLIENT_SECRET
    redirectURIs:
      - app.immich:///oauth-callback
      - $IMMICH_URL/auth/login
      - $IMMICH_URL/user-settings
staticPasswords:
  - email: admin@example.com
    hash: "$HASH"
    username: admin
    userID: 08a8684b-db88-4b73-90a9-3cd1661f5466
EOF
in_ct sh -c 'docker rm -f dex >/dev/null 2>&1 || true'
in_ct docker run -d --name dex --restart unless-stopped -p 5556:5556 -v /opt/dex:/etc/dex:ro "$DEX_IMAGE" \
  dex serve /etc/dex/config.yaml >/dev/null
wait_for "$P" "$NAME" "Dex's discovery document" curl -fsS -o /dev/null "http://127.0.0.1:5556/dex/.well-known/openid-configuration"

step "immich: point OAuth at Dex"
# From inside the server's container, so it works in NAT mode too, where this host cannot reach the server.
CONFIG=$(in_ct curl -fsS -H "x-api-key: $IMMICH_API_KEY_FULL" "http://127.0.0.1:2283/api/system-config")
printf '%s' "$CONFIG" | jq --arg issuer "$ISSUER" --arg secret "$CLIENT_SECRET" '.oauth += {
    enabled: true, issuerUrl: $issuer, clientId: "immich", clientSecret: $secret,
    scope: "openid email profile", autoRegister: true, buttonText: "Sign in with Dex",
    tokenEndpointAuthMethod: "client_secret_post", mobileOverrideEnabled: false,
    allowInsecureRequests: true
  }' |
  inc exec --project "$P" "$NAME" -- curl -fsS -o /dev/null -X PUT -H "x-api-key: $IMMICH_API_KEY_FULL" \
    -H 'content-type: application/json' --data-binary @- "http://127.0.0.1:2283/api/system-config"
in_ct curl -fsS "http://127.0.0.1:2283/api/server/features" | jq -e '.oauth == true' >/dev/null || die "Immich does not report OAuth"

grep -v '^DEX_' "$HERE/.env.local" > "$HERE/.env.local.tmp" || true
{ cat "$HERE/.env.local.tmp"; echo "DEX_ISSUER=$ISSUER"; echo "DEX_EMAIL=admin@example.com"; echo "DEX_PASSWORD=$DEX_PASSWORD"; } > "$HERE/.env.local"
rm -f "$HERE/.env.local.tmp"
chmod 600 "$HERE/.env.local"
step "done: OIDC via $ISSUER (user admin@example.com, password in .env.local)"
