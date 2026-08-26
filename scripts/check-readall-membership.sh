#!/usr/bin/env bash
# `read_all` (the read-only admin flag) must be a FLOOR, not a ceiling.
#
# role_in() used to return Viewer for any read_all user and stop, which threw away an
# explicit membership: you could make a read-only admin the OWNER of one workspace in
# the UI and they still got read-only there. Fixed — this pins both directions:
#   1. read_all alone  -> can read every workspace, cannot write to any
#   2. read_all + owner membership on ws A -> can write to A
#   3. ...and STILL cannot write to ws B, where they have no membership
#   4. read_all alone gets no exec; the capability needs an explicit owner membership
# Boots its own hub against throwaway databases. Idempotent + self-cleaning.
#   bash scripts/check-readall-membership.sh          (needs Docker)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG=vantage-ra-cfg-$$; DAT=vantage-ra-dat-$$
PORT=${PORT:-18092}
BASE="http://127.0.0.1:$PORT"
HUB=""
fail=0
say() { printf '%-56s ' "$1"; }
ok()  { echo "ok"; }
bad() { echo "FAIL ($1)"; fail=1; }
cleanup() {
  [ -n "$HUB" ] && { kill "$HUB" 2>/dev/null; wait "$HUB" 2>/dev/null; }
  docker rm -f "$CFG" "$DAT" >/dev/null 2>&1
}
trap cleanup EXIT

start_pg() {
  docker run -d --name "$1" -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage \
    -e POSTGRES_DB="$3" -p "$4":5432 "$2" >/dev/null
  local o=0
  for _ in $(seq 1 90); do
    if docker exec "$1" pg_isready -U vantage >/dev/null 2>&1; then o=$((o+1)); [ $o -ge 4 ] && return 0; else o=0; fi
    sleep 1
  done
  echo "postgres $1 never became ready"; exit 1
}
echo "starting throwaway databases…"
start_pg "$CFG" postgres:16.6-alpine vantage_config 55472
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55473

echo "building + starting the hub…"
( cd "$REPO" && bash scripts/frontend.sh build >/dev/null 2>&1 )
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55472/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55473/vantage_data \
BIND_ADDR="127.0.0.1:$PORT" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
INSECURE_COOKIES=1 RUST_LOG=warn cargo run -q --release -p vantage-hub >/tmp/readall-hub.log 2>&1 &
HUB=$!
for _ in $(seq 1 300); do curl -s -o /dev/null "$BASE/healthz" && break; kill -0 $HUB 2>/dev/null || { echo "hub died:"; tail -20 /tmp/readall-hub.log; exit 1; }; sleep 1; done

py() { python3 -c "$1"; }
AJAR=$(mktemp); RJAR=$(mktemp)
trap 'cleanup; rm -f "$AJAR" "$RJAR"' EXIT
curl -s -c "$AJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"admin@local","password":"admin123"}'

mkws() { curl -s -b "$AJAR" -X POST "$BASE/api/workspaces" -H 'content-type: application/json' \
  -d "{\"name\":\"$1\"}" | py "import sys,json;print(json.load(sys.stdin)['id'])"; }
WSA=$(mkws ra-alpha); WSB=$(mkws ra-bravo)
[ -n "$WSA" ] && [ -n "$WSB" ] || { echo "could not create workspaces"; exit 1; }

# a read-only admin with NO memberships
RMAIL=readall-probe@example.com; RPASS='Readall-probe-pw-123!'
curl -s -b "$AJAR" -o /dev/null -X POST "$BASE/api/users" -H 'content-type: application/json' \
  -d "{\"email\":\"$RMAIL\",\"password\":\"$RPASS\",\"read_all\":true}"
RUID=$(curl -s -b "$AJAR" "$BASE/api/users" \
  | py "import sys,json;d=[u for u in json.load(sys.stdin) if u['email']=='$RMAIL'];print(d[0]['id'] if d else '')")
[ -n "$RUID" ] || { echo "could not create the read_all probe user"; exit 1; }
curl -s -c "$RJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"$RMAIL\",\"password\":\"$RPASS\"}"

# write probe: creating a notify channel needs Editor on the workspace
wcode() { curl -s -b "$RJAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$1/channels" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"probe-$2\",\"kind\":\"webhook\",\"config\":{\"url\":\"https://example.com/hook\"}}"; }
rcode() { curl -s -b "$RJAR" -o /dev/null -w '%{http_code}' "$BASE/api/workspaces/$1/alerts"; }

echo
echo "1) read_all alone"
say "reads workspace A"; c=$(rcode "$WSA"); [ "$c" = 200 ] && ok || bad "$c"
say "reads workspace B"; c=$(rcode "$WSB"); [ "$c" = 200 ] && ok || bad "$c"
say "cannot write to A"; c=$(wcode "$WSA" a1); [ "$c" = 403 ] && ok || bad "$c"
say "cannot write to B"; c=$(wcode "$WSB" b1); [ "$c" = 403 ] && ok || bad "$c"

echo
echo "2) read_all + explicit owner membership on A"
say "grant owner on A"
c=$(curl -s -b "$AJAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/workspaces/$WSA/members" \
  -H 'content-type: application/json' -d "{\"email\":\"$RMAIL\",\"role\":\"owner\"}")
case "$c" in 2*) ok;; *) bad "$c";; esac
say "CAN now write to A (the whole point)"
c=$(wcode "$WSA" a2); case "$c" in 2*) ok;; *) bad "$c — membership is being discarded";; esac
say "still cannot write to B"
c=$(wcode "$WSB" b2); [ "$c" = 403 ] && ok || bad "$c — read_all must not grant writes"

echo
echo "3) exec is never implied by read_all"
SYS=$(docker exec -i "$CFG" psql -tAqX -U vantage -d vantage_config -c \
  "INSERT INTO api_keys (workspace_id, name, key) VALUES ('$WSB','ra','ra-probe-key') RETURNING id;" 2>/dev/null | tr -d '[:space:]')
SYSID=$(docker exec -i "$CFG" psql -tAqX -U vantage -d vantage_config -c \
  "INSERT INTO systems (workspace_id, key_id, name, hostname, kind, agent_version, last_seen) \
   VALUES ('$WSB','$SYS','ra-host','ra-host','node','9.9.9',now()) RETURNING id;" 2>/dev/null | tr -d '[:space:]')
if [ -n "$SYSID" ]; then
  say "no console ticket on a workspace they only read"
  c=$(curl -s -b "$RJAR" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/systems/$SYSID/console/ticket" \
    -H 'content-type: application/json' -d '{"auth":"password","ssh_user":"probe","ssh_password":"x"}')
  [ "$c" = 403 ] && ok || bad "$c"
else
  echo "  (skipped: could not seed a system)"
fi

echo "─────────────────────────────────────────────────────────"
[ "$fail" = 0 ] && echo "PASS — read_all is a floor; memberships still raise it" || echo "FAILED"
exit "$fail"
