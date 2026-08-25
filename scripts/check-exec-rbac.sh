#!/usr/bin/env bash
# The shell/console tunnel is ON by default on the agent side (3.0.12), so the ONLY thing
# standing between a logged-in user and a root shell on your hosts is the exec capability.
# This check pins that gate down, plus the chart rendering that makes "just upgrade the
# agent" work:
#   1. the agent chart emits NO ALLOW_SHELL env by default (binary default governs, so no
#      stale ALLOW_SHELL=0 survives in an upgraded pod spec) and emits "0" only on opt-out
#   2. /pub/agent.yaml ships ALLOW_SHELL=1
#   3. an OWNER without can_exec           -> 403 on the console ticket
#   4. an owner WITH can_exec              -> passes the gate (400 "agent offline", not 403)
#   5. a VIEWER with can_exec              -> still 403 (require_exec needs owner + can_exec)
#   6. GET /api/systems/:id/shell reports can_exec honestly to each caller
# Boots its own hub against throwaway databases. Idempotent + self-cleaning.
#   bash scripts/check-exec-rbac.sh          (needs Docker)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG=vantage-exec-cfg-$$; DAT=vantage-exec-dat-$$
PORT=${PORT:-18091}
BASE="http://127.0.0.1:$PORT"
HUB=""
fail=0
say() { printf '%-52s ' "$1"; }
ok()  { echo "ok"; }
bad() { echo "FAIL ($1)"; fail=1; }
cleanup() {
  [ -n "$HUB" ] && { kill "$HUB" 2>/dev/null; wait "$HUB" 2>/dev/null; }
  docker rm -f "$CFG" "$DAT" >/dev/null 2>&1
}
trap cleanup EXIT

echo "1) chart + installer rendering (no hub needed)"
HELM_ARGS=(--set hubUrl=https://hub.example.com --set apiKey=k)
say "agent chart: no ALLOW_SHELL env by default"
n=$(helm template t "$REPO/deploy/agent" "${HELM_ARGS[@]}" | grep -c "name: ALLOW_SHELL")
[ "$n" = 0 ] && ok || bad "$n env entries — a stale 0 would survive an image upgrade"
say "agent chart: allowShell=false emits ALLOW_SHELL=0"
out=$(helm template t "$REPO/deploy/agent" "${HELM_ARGS[@]}" --set allowShell=false | grep -A0 "name: ALLOW_SHELL")
printf '%s' "$out" | grep -q 'value: "0"' && ok || bad "got: $out"
say "/pub/agent.yaml installer ships ALLOW_SHELL=1"
grep -A1 'name: ALLOW_SHELL' "$REPO/crates/hub/templates/agent.yaml.tmpl" | grep -q 'value: "1"' && ok || bad "installer still opts out"

start_pg() { # name image db hostport
  docker run -d --name "$1" -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage \
    -e POSTGRES_DB="$3" -p "$4":5432 "$2" >/dev/null
  local o=0
  for _ in $(seq 1 90); do
    if docker exec "$1" pg_isready -U vantage >/dev/null 2>&1; then o=$((o+1)); [ $o -ge 4 ] && return 0; else o=0; fi
    sleep 1
  done
  echo "postgres $1 never became ready"; exit 1
}
echo
echo "starting throwaway databases…"
start_pg "$CFG" postgres:16.6-alpine vantage_config 55462
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55463

echo "building + starting the hub…"
( cd "$REPO" && bash scripts/frontend.sh build >/dev/null 2>&1 )
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55462/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55463/vantage_data \
BIND_ADDR="127.0.0.1:$PORT" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
INSECURE_COOKIES=1 RUST_LOG=warn cargo run -q --release -p vantage-hub >/tmp/execrbac-hub.log 2>&1 &
HUB=$!
for _ in $(seq 1 300); do curl -s -o /dev/null "$BASE/healthz" && break; kill -0 $HUB 2>/dev/null || { echo "hub died:"; tail -20 /tmp/execrbac-hub.log; exit 1; }; sleep 1; done

py() { python3 -c "$1"; }
AJAR=$(mktemp); OJAR=$(mktemp); VJAR=$(mktemp)
trap 'cleanup; rm -f "$AJAR" "$OJAR" "$VJAR"' EXIT
curl -s -c "$AJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"admin@local","password":"admin123"}'

# workspace + API key + a system row to aim the console at
WS=$(curl -s -b "$AJAR" -X POST "$BASE/api/workspaces" -H 'content-type: application/json' \
  -d '{"name":"execprobe"}' | py "import sys,json;print(json.load(sys.stdin)['id'])")
curl -s -b "$AJAR" -o /dev/null -X POST "$BASE/api/workspaces/$WS/keys" \
  -H 'content-type: application/json' -d '{"name":"execprobe"}'
SYS=$(docker exec -i "$CFG" psql -tAqX -U vantage -d vantage_config -c \
  "INSERT INTO systems (workspace_id, key_id, name, hostname, kind, agent_version, last_seen) \
   SELECT k.workspace_id, k.id, 'execprobe-host','execprobe-host','node','9.9.9',now() \
   FROM api_keys k WHERE k.workspace_id = '$WS' LIMIT 1 RETURNING id;" | tr -d '[:space:]')
[ -n "$SYS" ] || { echo "could not seed a system"; exit 1; }

mkuser() { # email password role
  curl -s -b "$AJAR" -o /dev/null -X POST "$BASE/api/users" -H 'content-type: application/json' \
    -d "{\"email\":\"$1\",\"password\":\"$2\"}"
  curl -s -b "$AJAR" -o /dev/null -X POST "$BASE/api/workspaces/$WS/members" \
    -H 'content-type: application/json' -d "{\"email\":\"$1\",\"role\":\"$3\"}"
  curl -s -b "$AJAR" "$BASE/api/users" \
    | py "import sys,json;d=[u for u in json.load(sys.stdin) if u['email']=='$1'];print(d[0]['id'] if d else '')"
}
OUID=$(mkuser owner-probe@example.com 'Owner-probe-pw-123!' owner)
VUID=$(mkuser viewer-probe@example.com 'Viewer-probe-pw-123!' viewer)
[ -n "$OUID" ] && [ -n "$VUID" ] || { echo "could not create probe users"; exit 1; }
curl -s -c "$OJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"owner-probe@example.com","password":"Owner-probe-pw-123!"}'
curl -s -c "$VJAR" -o /dev/null -X POST "$BASE/api/auth/login" -H 'content-type: application/json' \
  -d '{"email":"viewer-probe@example.com","password":"Viewer-probe-pw-123!"}'

ticket() { # jar -> http code
  curl -s -b "$1" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/systems/$SYS/console/ticket" \
    -H 'content-type: application/json' \
    -d '{"auth":"password","ssh_user":"probe","ssh_password":"whatever"}'
}
grant() { # user_id true|false
  curl -s -b "$AJAR" -o /dev/null -w '%{http_code}' -X PUT "$BASE/api/workspaces/$WS/members/$1/exec" \
    -H 'content-type: application/json' -d "{\"can_exec\":$2}"
}
shell_can_exec() { curl -s -b "$1" "$BASE/api/systems/$SYS/shell" | py "import sys,json;print(json.load(sys.stdin).get('can_exec'))"; }

echo
echo "2) the exec capability gate"
say "owner WITHOUT can_exec -> 403"
c=$(ticket "$OJAR"); [ "$c" = 403 ] && ok || bad "$c"
say "  and /shell reports can_exec=false"
c=$(shell_can_exec "$OJAR"); [ "$c" = "False" ] && ok || bad "$c"

say "grant can_exec to the owner"
c=$(grant "$OUID" true); case "$c" in 2*) ok;; *) bad "$c";; esac
say "owner WITH can_exec -> past the gate (400, agent offline)"
c=$(ticket "$OJAR"); [ "$c" = 400 ] && ok || bad "$c — 403 means the grant did not take"
say "  and /shell reports can_exec=true"
c=$(shell_can_exec "$OJAR"); [ "$c" = "True" ] && ok || bad "$c"

say "viewer WITH can_exec -> still 403 (needs owner)"
grant "$VUID" true >/dev/null
c=$(ticket "$VJAR"); [ "$c" = 403 ] && ok || bad "$c — a viewer must never reach a shell"
say "  and /shell reports can_exec=false to the viewer"
c=$(shell_can_exec "$VJAR"); [ "$c" = "False" ] && ok || bad "$c"

say "revoking can_exec closes it again -> 403"
grant "$OUID" false >/dev/null
c=$(ticket "$OJAR"); [ "$c" = 403 ] && ok || bad "$c"

say "anonymous caller -> 401"
c=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/api/systems/$SYS/console/ticket" \
  -H 'content-type: application/json' -d '{"auth":"password","ssh_user":"probe","ssh_password":"x"}')
[ "$c" = 401 ] && ok || bad "$c"

echo "─────────────────────────────────────────────────────"
[ "$fail" = 0 ] && echo "PASS — tunnel on by default, exec still gated by owner + can_exec" || echo "FAILED"
exit "$fail"
