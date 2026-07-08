#!/usr/bin/env bash
# Apply every migrations/config/*.sql in order against a throwaway Postgres and
# assert the schema converges on the "workspace" naming (0027 rename). Guards the
# append-only rename: migrations 0001-0017 keep "namespace" naming so old DBs still
# validate, and 0027 renames forward. Idempotent + self-cleaning.
#   bash scripts/check-config-migrations.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CID="vantage-migtest-$$"
cleanup() { docker rm -f "$CID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "starting throwaway Postgres…"
docker run -d --name "$CID" \
  -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage -e POSTGRES_DB=vantage_config \
  timescale/timescaledb:latest-pg18 >/dev/null

echo "waiting for readiness…"
# initdb brings up a temporary server (local socket) to run init, then restarts to
# listen for real — so a single successful check can hit the soon-to-stop temp server.
# Require several consecutive real-query successes to be sure we're on the final server.
ok=0
for i in $(seq 1 90); do
  if docker exec "$CID" psql -tAqX -U vantage -d vantage_config -c "SELECT 1" >/dev/null 2>&1; then
    ok=$((ok + 1)); [ "$ok" -ge 4 ] && break
  else
    ok=0
  fi
  sleep 1
  [ "$i" = 90 ] && { echo "postgres never became stably ready"; exit 1; }
done

echo "applying config migrations in order…"
for f in "$REPO"/migrations/config/*.sql; do
  name="$(basename "$f")"
  if ! docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_config < "$f" >/dev/null 2>/tmp/mig_err; then
    echo "FAIL applying $name:"; cat /tmp/mig_err; exit 1
  fi
done
echo "all $(ls "$REPO"/migrations/config/*.sql | wc -l | tr -d ' ') migrations applied"

q() { docker exec "$CID" psql -tAqX -U vantage -d vantage_config -c "$1"; }

echo "asserting converged schema…"
fail=0
assert() { # <desc> <actual> <expected>
  if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1 (got '$2', want '$3')"; fail=1; fi
}
assert "workspaces table exists"      "$(q "SELECT to_regclass('workspaces') IS NOT NULL;")" "t"
assert "namespaces table gone"        "$(q "SELECT to_regclass('namespaces') IS NULL;")"      "t"
assert "ws_role type exists"          "$(q "SELECT EXISTS(SELECT 1 FROM pg_type WHERE typname='ws_role');")" "t"
assert "ns_role type gone"            "$(q "SELECT NOT EXISTS(SELECT 1 FROM pg_type WHERE typname='ns_role');")" "t"
for t in memberships api_keys systems monitors channels status_pages exec_sessions; do
  assert "$t.workspace_id exists" \
    "$(q "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='$t' AND column_name='workspace_id');")" "t"
  assert "$t.namespace_id gone" \
    "$(q "SELECT NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='$t' AND column_name='namespace_id');")" "t"
done
assert "alerts.scope_workspace_id exists" \
  "$(q "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='alerts' AND column_name='scope_workspace_id');")" "t"
for idx in idx_systems_workspace idx_monitors_workspace idx_exec_sessions_workspace; do
  assert "index $idx exists" "$(q "SELECT to_regclass('$idx') IS NOT NULL;")" "t"
done
assert "default workspace row seeded" "$(q "SELECT count(*) FROM workspaces WHERE name='default';")" "1"

[ "$fail" = 0 ] && echo "PASS — config migrations converge on workspace schema" || { echo "FAILED"; exit 1; }
