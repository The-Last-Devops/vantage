#!/usr/bin/env bash
# Cloudflare Access JWT on /mcp: boot the hub on throwaway databases with
# CF_ACCESS_TEAM_DOMAIN/CF_ACCESS_AUD set, then confirm
#   · PATs are untouched (runs check-pat.sh + check-mcp.sh against it),
#   · a forged Cf-Access-Jwt-Assertion is rejected on /mcp,
#   · the JWT is NOT honoured outside MCP (/api/*),
#   · the GET /mcp self-check advertises Cloudflare Access.
# Signature/claims are covered by unit tests (cf_access::tests); a real Access JWT
# can't be minted here, so the positive path is verified in production.
#
#   bash scripts/check-cf-access.sh        (needs Docker)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG=vantage-cfa-cfg-$$; DAT=vantage-cfa-dat-$$
PORT=${PORT:-18091}
BASE="http://127.0.0.1:$PORT"
LOG="$(mktemp)"
HUB=""
cleanup() {
  [ -n "$HUB" ] && { kill "$HUB" 2>/dev/null; wait "$HUB" 2>/dev/null; }
  docker rm -f "$CFG" "$DAT" >/dev/null 2>&1
  rm -f "$LOG"
}
trap cleanup EXIT INT TERM

start_pg() { # name image db hostport
  docker run -d --name "$1" -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage \
    -e POSTGRES_DB="$3" -p "$4":5432 "$2" >/dev/null
  local ok=0
  for _ in $(seq 1 90); do
    if docker exec "$1" pg_isready -U vantage >/dev/null 2>&1; then ok=$((ok+1)); [ $ok -ge 4 ] && return 0; else ok=0; fi
    sleep 1
  done
  echo "postgres $1 never became ready"; exit 1
}

echo "starting throwaway databases…"
start_pg "$CFG" postgres:16.6-alpine vantage_config 55462
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55463

echo "building + starting the hub (CF Access enabled)…"
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55462/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55463/vantage_data \
BIND_ADDR="127.0.0.1:$PORT" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
CF_ACCESS_TEAM_DOMAIN="${CF_ACCESS_TEAM_DOMAIN:-example.cloudflareaccess.com}" \
CF_ACCESS_AUD="${CF_ACCESS_AUD:-aud-check}" \
INSECURE_COOKIES=1 RUST_LOG=info,sqlx=warn cargo run -q -p vantage-hub >"$LOG" 2>&1 &
HUB=$!
for _ in $(seq 1 600); do
  curl -s -o /dev/null "$BASE/healthz" && break
  kill -0 $HUB 2>/dev/null || { echo "hub died:"; tail -20 "$LOG"; exit 1; }
  sleep 1
done

fail=0
say() { printf '%-48s ' "$1"; }
chk() { say "$1"; if [ "$2" = "$3" ]; then echo "ok ($3)"; else echo "FAIL want $2 got $3"; fail=1; fi; }

echo "--- check-pat.sh";  BASE="$BASE" bash scripts/check-pat.sh || fail=1
echo "--- check-mcp.sh";  BASE="$BASE" bash scripts/check-mcp.sh || fail=1
echo "--- Cloudflare Access"

say "hub logged CF Access enabled"
grep -q "Cloudflare Access JWT enabled" "$LOG" && echo ok || { echo FAIL; fail=1; }

# RS256 header with an unknown kid + junk signature: must never authenticate.
FORGED='eyJhbGciOiJSUzI1NiIsImtpZCI6ImZvcmdlZCJ9.eyJlbWFpbCI6ImFkbWluQGxvY2FsIiwiZXhwIjo0MTAyNDQ0ODAwfQ.c2ln'
NONE='eyJhbGciOiJub25lIn0.eyJlbWFpbCI6ImFkbWluQGxvY2FsIiwiZXhwIjo0MTAyNDQ0ODAwfQ.'
RPC='{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
for t in "$FORGED" "$NONE"; do
  chk "forged JWT rejected on POST /mcp" 401 \
    "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/mcp" -H 'content-type: application/json' \
       -H "Cf-Access-Jwt-Assertion: $t" -d "$RPC")"
done
chk "JWT ignored outside MCP (/api/systems)" 401 \
  "$(curl -s -o /dev/null -w '%{http_code}' -H "Cf-Access-Jwt-Assertion: $FORGED" "$BASE/api/systems")"

SELF=$(curl -s -H "Cf-Access-Jwt-Assertion: $FORGED" "$BASE/mcp")
say "self-check advertises Cloudflare Access"
printf '%s' "$SELF" | grep -q 'Cloudflare Access' && echo ok || { echo "FAIL ($SELF)"; fail=1; }
say "self-check: forged JWT is unauthenticated"
printf '%s' "$SELF" | grep -q '"authenticated":false' && echo ok || { echo "FAIL ($SELF)"; fail=1; }

say "hub still alive"
kill -0 $HUB 2>/dev/null && echo ok || { echo FAIL; fail=1; }

[ $fail -eq 0 ] && echo "ALL OK" || { echo "FAILED"; tail -20 "$LOG"; }
exit $fail
