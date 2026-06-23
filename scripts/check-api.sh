#!/usr/bin/env bash
# Smoke-check the local dev hub's HTTP API without typing curl one-liners
# (so the agent never has to ask to run ad-hoc curl). Logs in as the dev admin,
# then probes the endpoints touched by the new IA: about, audit, alerts.
#
#   bash scripts/check-api.sh
#
# Override target/creds via env: BASE, ADMIN_EMAIL, ADMIN_PASSWORD.
set -euo pipefail
BASE="${BASE:-http://localhost:8080}"
EMAIL="${ADMIN_EMAIL:-admin@local}"
PASS="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

say() { printf '\n=== %s ===\n' "$1"; }

say "healthz"
curl -fsS "$BASE/healthz" && echo

say "login"
curl -fsS -c "$JAR" -X POST "$BASE/api/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" && echo

say "GET /api/about"
curl -fsS -b "$JAR" "$BASE/api/about"; echo

say "GET /api/namespaces"
NS=$(curl -fsS -b "$JAR" "$BASE/api/namespaces")
echo "$NS"
NS_ID=$(printf '%s' "$NS" | sed -n 's/.*"id":"\([0-9a-f-]*\)".*/\1/p' | head -1)

say "GET /api/audit (admin)"
curl -fsS -b "$JAR" "$BASE/api/audit" | head -c 600; echo

if [ -n "${NS_ID:-}" ]; then
  say "GET /api/namespaces/$NS_ID/alerts (expects condition field)"
  curl -fsS -b "$JAR" "$BASE/api/namespaces/$NS_ID/alerts"; echo
fi

echo
echo "OK"
