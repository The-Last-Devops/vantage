#!/usr/bin/env bash
# Fresh-install smoke test for the REAL sqlx migrator (not psql): boots throwaway
# config + data Postgres containers, starts the hub against them and asserts it gets
# past "migrations applied".
#
# Guards the bug that broke `helm install` on a fresh cluster: the squashed config
# 0001 came from pg_dump, which blanks search_path for its session. sqlx applies a
# migration and records it in _sqlx_migrations on the SAME connection, so the blank
# search_path made startup die with:
#     relation "_sqlx_migrations" does not exist
# psql-based checks can't see it (each file is its own psql session).
#
#   bash scripts/check-fresh-install.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG="vantage-fresh-cfg-$$"; DAT="vantage-fresh-dat-$$"
LOG="$(mktemp)"
cleanup() { docker rm -f "$CFG" "$DAT" >/dev/null 2>&1 || true; rm -f "$LOG"; }
trap cleanup EXIT

grep -q "set_config('search_path', '', false)" "$REPO"/migrations/config/*.sql \
  && { echo "FAIL: a config migration blanks search_path — sqlx cannot record _sqlx_migrations"; exit 1; }

start_pg() { # <name> <image> <db> <hostport>
  docker run -d --name "$1" -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage \
    -e POSTGRES_DB="$3" -p "$4":5432 "$2" >/dev/null
  local ok=0
  for _ in $(seq 1 90); do
    if docker exec "$1" psql -tAqX -U vantage -d "$3" -c "SELECT 1" >/dev/null 2>&1; then
      ok=$((ok+1)); [ "$ok" -ge 4 ] && return 0
    else ok=0; fi
    sleep 1
  done
  echo "postgres $1 never became ready"; exit 1
}

echo "starting throwaway databases…"
start_pg "$CFG" postgres:16.6-alpine vantage_config 55432
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55433

echo "starting the hub (real sqlx migrator)…"
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55432/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55433/vantage_data \
BIND_ADDR=127.0.0.1:18080 ADMIN_EMAIL=admin@local INSECURE_COOKIES=1 \
  cargo run -q -p vantage-hub >"$LOG" 2>&1 &
HUB=$!
trap 'kill $HUB 2>/dev/null || true; cleanup' EXIT

for _ in $(seq 1 180); do
  grep -q "migrations applied" "$LOG" && break
  kill -0 $HUB 2>/dev/null || { echo "FAIL: hub exited during startup:"; cat "$LOG"; exit 1; }
  sleep 1
done
grep -q "migrations applied" "$LOG" || { echo "FAIL: migrations never applied:"; cat "$LOG"; exit 1; }

echo "  ✓ migrations applied on a fresh install"
{ kill $HUB && wait $HUB; } 2>/dev/null || true
