#!/usr/bin/env bash
# Regression check: a push monitor's push_token must survive an edit (PATCH) that
# sends a config WITHOUT the token (the edit form rebuilds config from fields).
#
#   bash scripts/check-push-token.sh
set -euo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT

curl -fsS -c "$JAR" -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" >/dev/null

WS=$(curl -fsS -b "$JAR" "$BASE/api/workspaces")
WS_ID=$(printf '%s' "$WS" | sed -n 's/.*"id":"\([0-9a-f-]*\)".*/\1/p' | head -1)

echo "create push monitor…"
MID=$(curl -fsS -b "$JAR" -X POST "$BASE/api/workspaces/$WS_ID/monitors" \
  -H 'content-type: application/json' \
  -d '{"name":"__pushcheck","kind":"push","target":"push","interval_secs":60}' | tr -d '"')

tok() { curl -fsS -b "$JAR" "$BASE/api/monitors" \
  | sed -n "s/.*\"id\":\"$MID\"[^}]*\"push_token\":\"\([0-9a-f]*\)\".*/\1/p" | head -1; }
T1=$(tok); echo "token after create: ${T1:-<empty>}"

echo "edit (PATCH config WITHOUT push_token)…"
curl -fsS -b "$JAR" -X PATCH "$BASE/api/monitors/$MID" -H 'content-type: application/json' \
  -d '{"name":"__pushcheck","config":{"timeout_secs":15,"retries":0,"upside_down":false}}' -o /dev/null
T2=$(tok); echo "token after edit:   ${T2:-<empty>}"

curl -fsS -b "$JAR" -X DELETE "$BASE/api/monitors/$MID" -o /dev/null  # cleanup

if [ -n "$T1" ] && [ "$T1" = "$T2" ]; then echo "PASS: token preserved across edit"; else echo "FAIL: token changed/lost ($T1 -> $T2)"; exit 1; fi
