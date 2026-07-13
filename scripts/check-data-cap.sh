#!/usr/bin/env bash
# Smoke-test the Data-DB storage cap: logs in as admin, checks that /api/admin/data
# reports the cap + per-tier size_bytes, then exercises the manual eviction endpoint
# POST /api/admin/data-cap/enforce and asserts its result shape.
#
#   bash scripts/check-data-cap.sh
#
# SAFE by design: the enforce endpoint is a NO-OP when the cap is disabled or the DB
# is already under the limit, so this never deletes real data. It verifies the wiring
# + response, not that eviction reclaims space when over cap — to test that end-to-end,
# point it at a disposable stack (scripts/reset-stack.sh) with a tiny cap set + enabled.
#
# Env overrides: BASE (default http://localhost:8080), ADMIN_EMAIL, ADMIN_PASSWORD.
set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"
JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

fail() { echo "✗ FAIL: $1"; [ -n "${2:-}" ] && echo "$2"; exit 1; }

echo "→ login as $ADMIN_EMAIL"
curl -sf -c "$JAR" -H 'Content-Type: application/json' \
  -d "{\"email\":\"$ADMIN_EMAIL\",\"password\":\"$ADMIN_PASSWORD\"}" \
  "$BASE/api/auth/login" >/dev/null || fail "login failed"

echo "→ GET /api/admin/data (cap + size_bytes present?)"
DATA="$(curl -sf -b "$JAR" "$BASE/api/admin/data")" || fail "GET /api/admin/data failed"
echo "$DATA" | grep -q '"cap"'         || fail "missing cap status" "$DATA"
echo "$DATA" | grep -q '"limit_bytes"' || fail "cap missing limit_bytes" "$DATA"
echo "$DATA" | grep -q '"size_bytes"'  || fail "tiers missing size_bytes (UI can't sort/colour)" "$DATA"

echo "→ POST /api/admin/data-cap/enforce (no-op safe)"
EV="$(curl -sf -b "$JAR" -X POST -H 'Content-Type: application/json' -d '{}' \
  "$BASE/api/admin/data-cap/enforce")" || fail "enforce endpoint failed"
for f in '"enabled"' '"limit_bytes"' '"used_bytes"' '"freed_bytes"' '"dropped_chunks"'; do
  echo "$EV" | grep -q "$f" || fail "enforce result missing $f" "$EV"
done
echo "  result: $EV"

echo "✓ PASS — cap + size_bytes exposed and the enforce endpoint responds"
