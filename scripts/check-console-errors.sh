#!/usr/bin/env bash
# Smoke-test the SPA in a real (headless) browser: boot the hub against throwaway
# databases, seed a k8s cluster + node hosts, open every main route and FAIL on any
# console error or uncaught exception.
#
# Catches what `vite build` cannot: valid-JS-but-broken code. 3.0.7 shipped
# `const usageCls = (p) = (...)` — an assignment, not an arrow function — which threw
# "ReferenceError: p is not defined" and blanked the whole cluster page.
#
#   bash scripts/check-console-errors.sh          (needs Docker + Google Chrome)
set -uo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CFG=vantage-cons-cfg-$$; DAT=vantage-cons-dat-$$
PORT=${PORT:-18090}
BASE="http://127.0.0.1:$PORT"
HUB=""
cleanup() {
  [ -n "$HUB" ] && { kill "$HUB" 2>/dev/null; wait "$HUB" 2>/dev/null; }
  docker rm -f "$CFG" "$DAT" >/dev/null 2>&1
  rm -f "$REPO/scripts/.sim-tokens.json"
}
trap cleanup EXIT

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
start_pg "$CFG" postgres:16.6-alpine vantage_config 55452
start_pg "$DAT" timescale/timescaledb:2.17.2-pg16 vantage_data 55453

echo "building + starting the hub…"
( cd "$REPO" && bash scripts/frontend.sh build >/dev/null 2>&1 )
cd "$REPO"
CONFIG_DATABASE_URL=postgres://vantage:vantage@localhost:55452/vantage_config \
DATA_DATABASE_URL=postgres://vantage:vantage@localhost:55453/vantage_data \
BIND_ADDR="127.0.0.1:$PORT" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
INSECURE_COOKIES=1 RUST_LOG=warn cargo run -q --release -p vantage-hub >/tmp/consolecheck-hub.log 2>&1 &
HUB=$!
for _ in $(seq 1 300); do curl -s -o /dev/null "$BASE/healthz" && break; kill -0 $HUB 2>/dev/null || { echo "hub died:"; tail -20 /tmp/consolecheck-hub.log; exit 1; }; sleep 1; done

echo "seeding a fleet (k8s nodes + a cluster with pod stats)…"
HUB_URL="$BASE" ADMIN_EMAIL=admin@local ADMIN_PASSWORD=admin123 \
  DURATION=8 NODES=1 DOCKER=1 K8S_CLUSTERS=1 K8S_NODES=3 INTERVAL=3 \
  node scripts/sim-agents.mjs >/dev/null 2>&1
CID=$(docker exec -i "$CFG" psql -tAqX -U vantage -d vantage_config -c \
  "INSERT INTO systems (workspace_id, key_id, name, hostname, kind, cluster, agent_version, k8s_version, last_seen) \
   SELECT k.workspace_id, k.id, 'cluster-1','cluster-1','k8s-cluster','cluster-1','9.9.9','v1.33.0',now() \
   FROM api_keys k LIMIT 1 RETURNING id;" 2>/dev/null | tr -d '[:space:]')
if [ -n "$CID" ]; then
  docker exec -i "$DAT" psql -q -U vantage -d vantage_data >/dev/null 2>&1 <<SQL
INSERT INTO kube_container_stats (time, system_id, namespace, pod, container, node, phase, workload, workload_kind, cpu_millicores, mem_bytes, restarts, labels)
SELECT now(), '$CID'::uuid, t.ns, t.pod, 'app', t.node, 'Running', 'web', 'Deployment', t.cpu, t.mem, 1, '{"app":"web"}'::jsonb
FROM unnest(ARRAY['default','kube-system','staging']::text[], ARRAY['p1','p2','p3']::text[],
            ARRAY['k8s1-cp-1','k8s1-worker-2','k8s1-worker-3']::text[],
            ARRAY[250,120,60]::bigint[], ARRAY[134217728,67108864,33554432]::bigint[]) AS t(ns,pod,node,cpu,mem);
SQL
fi

if [ -z "$CID" ]; then
  echo "FAIL: could not seed a k8s-cluster system (no api_keys row?) — the cluster page is"
  echo "      the main thing this check covers, so treat this as a failure."
  exit 1
fi
# A "##" suffix runs JS once the route settles — the member edit slide-over holds real
# logic (draft diffing, discard-on-close) that opening the route alone never executes.
ROUTES=(/ /clusters /services /alerts /events /channels /settings /members /workspaces /fleet /metrics /alerts/new
        "/members##document.querySelector('[data-t=edit]').click()"
        "/cluster/$CID?name=cluster-1"
        "/cluster/$CID?name=cluster-1&tab=nodes"
        "/cluster/$CID?name=cluster-1&sel=default")
echo "cluster under test: $CID"
echo "opening routes in headless Chrome…"
python3 "$REPO/scripts/console-errors.py" "$BASE" admin@local admin123 "${ROUTES[@]}"
rc=$?
echo "---------------------------------------------"
[ "$rc" = 0 ] && echo "OK - no console errors" || echo "FAILURES - see above"
exit "$rc"
