#!/usr/bin/env bash
# Smoke-test the Data & retention admin API: logs in as the admin, fetches
# /api/admin/data, and asserts it returns both DB groups (data + config) plus
# the cap status. Idempotent and self-cleaning (only reads; temp cookie jar removed).
#
#   bash scripts/check-data-admin.sh
#
# Env overrides: BASE (default http://localhost:8080), ADMIN_EMAIL, ADMIN_PASSWORD.
set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

echo "→ login as $ADMIN_EMAIL"
curl -sf -c "$JAR" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}" \
  "$BASE/api/auth/login" >/dev/null

echo "→ GET /api/admin/data"
RESP="$(curl -sf -b "$JAR" "$BASE/api/admin/data")"

fail() { echo "✗ FAIL: $1"; echo "$RESP"; exit 1; }
echo "$RESP" | grep -q '"data"'    || fail "missing data group"
echo "$RESP" | grep -q '"config"'  || fail "missing config group"
echo "$RESP" | grep -q '"cap"'     || fail "missing cap status"
echo "$RESP" | grep -q '"retention"' || fail "missing retention tiers"
echo "✓ PASS — data + config groups and cap present"
