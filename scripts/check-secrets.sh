#!/usr/bin/env bash
# Verify channel secrets are masked for viewers and shown to editors+.
# A read-only member must never see tokens/passwords in GET .../channels.
# Self-cleaning: removes the probe channel + viewer it creates.
#   bash scripts/check-secrets.sh
set -uo pipefail
BASE="${BASE:-http://localhost:8080}"
ADMIN_EMAIL_="${ADMIN_EMAIL:-admin@local}"
ADMIN_PASS="${ADMIN_PASSWORD:-admin123}"
SECRET="super-secret-bot-token-123"
VIEWER="viewer-probe@example.com"
VPASS="viewer-pass-123"
ADM="$(mktemp)"; VW="$(mktemp)"; trap 'rm -f "$ADM" "$VW"' EXIT
py() { python3 -c "$1"; }
say() { printf '%-44s ' "$1"; }
fail=0

for i in $(seq 1 60); do curl -s -o /dev/null -m 2 "$BASE/healthz" && break; sleep 1; done
curl -s -c "$ADM" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL_\",\"password\":\"$ADMIN_PASS\"}"
WS=$(curl -s -b "$ADM" "$BASE/api/workspaces" | py "import sys,json;d=json.load(sys.stdin);print(d[0]['id'] if d else '')")

# viewer user + viewer membership in ns0 (ignore 409 if it already exists)
curl -s -b "$ADM" -o /dev/null -X POST "$BASE/api/users" -H 'content-type: application/json' \
  -d "{\"email\":\"$VIEWER\",\"password\":\"$VPASS\"}"
curl -s -b "$ADM" -o /dev/null -X POST "$BASE/api/workspaces/$WS/members" -H 'content-type: application/json' \
  -d "{\"email\":\"$VIEWER\",\"role\":\"viewer\"}"

# a channel carrying a secret (telegram bot_token is type:secret)
CH=$(curl -s -b "$ADM" -X POST "$BASE/api/workspaces/$WS/channels" -H 'content-type: application/json' \
  -d "{\"name\":\"secret-probe\",\"kind\":\"telegram\",\"config\":{\"bot_token\":\"$SECRET\",\"chat_id\":\"123\"}}" \
  | py "import sys,json;print(json.load(sys.stdin))")

tok() { # read bot_token of the probe channel from a given cookie jar
  curl -s -b "$1" "$BASE/api/workspaces/$WS/channels" \
    | py "import sys,json;d=[c for c in json.load(sys.stdin) if c['id']=='$CH'];print(d[0]['config'].get('bot_token','') if d else 'MISSING')"
}

say "owner/admin sees the real secret"
[ "$(tok "$ADM")" = "$SECRET" ] && echo "ok" || { echo "FAIL got '$(tok "$ADM")'"; fail=1; }

curl -s -c "$VW" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$VIEWER\",\"password\":\"$VPASS\"}"
VT="$(tok "$VW")"
say "viewer gets the secret masked"
{ [ "$VT" != "$SECRET" ] && [ "$VT" != "MISSING" ] && [ -n "$VT" ]; } && echo "ok (masked: $VT)" || { echo "FAIL got '$VT'"; fail=1; }

# cleanup
curl -s -b "$ADM" -o /dev/null -X DELETE "$BASE/api/channels/$CH"
VID=$(curl -s -b "$ADM" "$BASE/api/users" | py "import sys,json;d=[u for u in json.load(sys.stdin) if u['email']=='$VIEWER'];print(d[0]['id'] if d else '')")
[ -n "$VID" ] && curl -s -b "$ADM" -o /dev/null -X DELETE "$BASE/api/users/$VID"

[ "$fail" -eq 0 ] && echo "OK" || { echo "secret-exposure regression"; exit 1; }
