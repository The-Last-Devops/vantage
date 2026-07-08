#!/usr/bin/env bash
# Validate the Kubernetes per-container stats SQL end-to-end against a throwaway
# TimescaleDB: apply the data migrations (incl. 0005), replicate the ingest UNNEST
# insert, then run the aggregate/containers/series read queries and assert results.
# Idempotent + self-cleaning.
#   bash scripts/check-kube-stats.sh
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CID="vantage-kubetest-$$"
SID="00000000-0000-0000-0000-000000000001"
cleanup() { docker rm -f "$CID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "starting throwaway TimescaleDB…"
docker run -d --name "$CID" \
  -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage -e POSTGRES_DB=vantage_data \
  timescale/timescaledb:latest-pg18 >/dev/null

echo "waiting for readiness…"
ok=0
for i in $(seq 1 90); do
  if docker exec "$CID" psql -tAqX -U vantage -d vantage_data -c "SELECT 1" >/dev/null 2>&1; then
    ok=$((ok + 1)); [ "$ok" -ge 4 ] && break
  else ok=0; fi
  sleep 1
  [ "$i" = 90 ] && { echo "postgres never became stably ready"; exit 1; }
done

echo "applying data migrations…"
for f in "$REPO"/migrations/data/*.sql; do
  if ! docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_data < "$f" >/dev/null 2>/tmp/kube_err; then
    echo "FAIL applying $(basename "$f"):"; cat /tmp/kube_err; exit 1
  fi
done

psql() { docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -tAqX -U vantage -d vantage_data "$@"; }

echo "simulating two ingest snapshots (UNNEST insert — same shape as ingest.rs)…"
# Containers: default/web-1{app,sidecar} (Deployment/web), kube-system/log-1 (DaemonSet/logger),
# and staging/web-9 — a SECOND "web" Deployment in another namespace (same name).
for age in "2 minutes" "0 minutes"; do
docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_data >/dev/null <<SQL
INSERT INTO kube_container_stats
  (time, system_id, namespace, pod, container, node, phase, workload, workload_kind, cpu_millicores, mem_bytes, restarts, labels)
SELECT now() - interval '$age', '$SID'::uuid,
       t.ns, t.pod, t.container, t.node, t.phase, t.workload, t.workload_kind, t.cpu, t.mem, t.restarts, t.labels::jsonb
FROM unnest(
  ARRAY['default','default','kube-system','staging']::text[],
  ARRAY['web-1','web-1','log-1','web-9']::text[],
  ARRAY['app','sidecar','log','app']::text[],
  ARRAY['n1','n1','n2','n3']::text[],
  ARRAY['Running','Running','Running','Running']::text[],
  ARRAY['web','web','logger','web']::text[],
  ARRAY['Deployment','Deployment','DaemonSet','Deployment']::text[],
  ARRAY[250,10,30,100]::bigint[],
  ARRAY[134217728,16777216,33554432,1048576]::bigint[],
  ARRAY[2,0,1,0]::int[],
  ARRAY['{"app":"web"}','{"app":"web"}','{"app":"logger"}','{"app":"web"}']::text[]
) AS t(ns,pod,container,node,phase,workload,workload_kind,cpu,mem,restarts,labels);
SQL
done

fail=0
assert() { if [ "$2" = "$3" ]; then echo "  ok: $1"; else echo "  FAIL: $1 (got '$2', want '$3')"; fi; [ "$2" = "$3" ] || fail=1; }

# Reusable "latest snapshot" CTE prefix.
L="WITH l AS (SELECT max(time) t FROM kube_container_stats WHERE system_id='$SID')"

echo "asserting read queries…"
# aggregate scoped to ns=default: cpu 260 (250+10), pods 1, containers 2
read -r cpu pods conts <<<"$(psql -c "$L SELECT sum(cpu_millicores), count(DISTINCT pod), count(*) \
  FROM kube_container_stats c, l WHERE c.system_id='$SID' AND c.time=l.t AND namespace='default';" | tr '|' ' ')"
assert "ns=default cpu" "$cpu" "260"
assert "ns=default pods" "$pods" "1"
assert "ns=default containers" "$conts" "2"

# by=workload keeps same-named deployments in different namespaces DISTINCT (issue #2):
# 'Deployment/web' must appear once per namespace (default + staging), not merged.
n=$(psql -c "$L SELECT count(*) FROM ( \
  SELECT namespace, (workload_kind||'/'||workload) grp FROM kube_container_stats c, l \
  WHERE c.system_id='$SID' AND c.time=l.t GROUP BY namespace, grp) x WHERE grp='Deployment/web';")
assert "Deployment/web distinct across namespaces" "$n" "2"
d=$(psql -c "$L SELECT sum(cpu_millicores) FROM kube_container_stats c, l \
  WHERE c.system_id='$SID' AND c.time=l.t AND workload='web' AND namespace='default';")
assert "Deployment/web in default cpu" "$d" "260"
s=$(psql -c "$L SELECT sum(cpu_millicores) FROM kube_container_stats c, l \
  WHERE c.system_id='$SID' AND c.time=l.t AND workload='web' AND namespace='staging';")
assert "Deployment/web in staging cpu" "$s" "100"

# aggregate by label app=logger -> 30
lbl=$(psql -c "$L SELECT sum(cpu_millicores) FROM kube_container_stats c, l \
  WHERE c.system_id='$SID' AND c.time=l.t AND labels->>'app'='logger';")
assert "label app=logger cpu" "$lbl" "30"

# containers filtered by ns=default -> 2 rows
c=$(psql -c "$L SELECT count(*) FROM kube_container_stats c, l WHERE c.system_id='$SID' AND c.time=l.t AND namespace='default';")
assert "containers ns=default rows" "$c" "2"

# series: >=1 bucket; latest-snapshot total cpu = 250+10+30+100 = 390
buckets=$(psql -c "SELECT count(*) FROM (SELECT time_bucket('1 minute', time) t, avg(scpu) cpu FROM \
  (SELECT time, sum(cpu_millicores) scpu FROM kube_container_stats WHERE system_id='$SID' GROUP BY time) s GROUP BY 1) x;")
[ "$buckets" -ge 1 ] && echo "  ok: series buckets ($buckets)" || { echo "  FAIL: series buckets 0"; fail=1; }
scpu=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats WHERE system_id='$SID' \
  AND time=(SELECT max(time) FROM kube_container_stats WHERE system_id='$SID');")
assert "latest snapshot total cpu" "$scpu" "390"

[ "$fail" = 0 ] && echo "PASS — kube_container_stats migration + ingest + read queries valid" || { echo "FAILED"; exit 1; }
