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

# Reusable "latest snapshot" predicate — the exact shape the hub ships (web/kube.rs):
# a FRESH time bound on both halves plus a scalar subquery (not a CTE), so TimescaleDB
# can exclude chunks instead of reading the whole hypertable.
FRESH="6 hours"
W="c.system_id='$SID' AND c.time > now() - interval '$FRESH' \
   AND c.time = (SELECT max(time) FROM kube_container_stats \
                 WHERE system_id='$SID' AND time > now() - interval '$FRESH')"

echo "asserting read queries…"
# aggregate scoped to ns=default: cpu 260 (250+10), pods 1, containers 2
read -r cpu pods conts <<<"$(psql -c "SELECT sum(cpu_millicores), count(DISTINCT pod), count(*) \
  FROM kube_container_stats c WHERE $W AND namespace='default';" | tr '|' ' ')"
assert "ns=default cpu" "$cpu" "260"
assert "ns=default pods" "$pods" "1"
assert "ns=default containers" "$conts" "2"

# by=workload keeps same-named deployments in different namespaces DISTINCT (issue #2):
# 'Deployment/web' must appear once per namespace (default + staging), not merged.
n=$(psql -c "SELECT count(*) FROM ( \
  SELECT namespace, (workload_kind||'/'||workload) grp FROM kube_container_stats c \
  WHERE $W GROUP BY namespace, grp) x WHERE grp='Deployment/web';")
assert "Deployment/web distinct across namespaces" "$n" "2"
d=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c \
  WHERE $W AND workload='web' AND namespace='default';")
assert "Deployment/web in default cpu" "$d" "260"
s=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c \
  WHERE $W AND workload='web' AND namespace='staging';")
assert "Deployment/web in staging cpu" "$s" "100"

# by=node (the Nodes table + "group by Node"): one group per node, cpu split
# n1=250+10, n2=30, n3=100 — and pods counted per node.
nn=$(psql -c "SELECT count(*) FROM (SELECT (CASE WHEN node='' THEN '—' ELSE node END) grp \
  FROM kube_container_stats c WHERE $W GROUP BY grp) x;")
assert "by=node group count" "$nn" "3"
n1=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c \
  WHERE $W AND node='n1';")
assert "node n1 cpu" "$n1" "260"
n1p=$(psql -c "SELECT count(DISTINCT pod) FROM kube_container_stats c \
  WHERE $W AND node='n1';")
assert "node n1 pods" "$n1p" "1"
n3=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c \
  WHERE $W AND node='n3';")
assert "node n3 cpu" "$n3" "100"

# aggregate by label app=logger -> 30
lbl=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c \
  WHERE $W AND labels->>'app'='logger';")
assert "label app=logger cpu" "$lbl" "30"

# containers filtered by ns=default -> 2 rows
c=$(psql -c "SELECT count(*) FROM kube_container_stats c WHERE $W AND namespace='default';")
assert "containers ns=default rows" "$c" "2"

# series-by (per-group overlay): the time_bucket()+group query must return rows for
# each group (regression: time_bucket was once fed a text param and 500'd).
sb=$(psql -c "SELECT count(*) FROM ( \
  SELECT tb, grp, avg(scpu)::float8 cpu FROM ( \
    SELECT time_bucket('1 minute', time) tb, time, \
           (namespace||' · '||workload_kind||'/'||workload) grp, \
           sum(cpu_millicores) scpu, sum(mem_bytes) smem \
    FROM kube_container_stats WHERE system_id='$SID' AND time > now() - interval '1 hour' \
    GROUP BY tb, time, grp) s GROUP BY tb, grp) x;")
[ "$sb" -ge 1 ] && echo "  ok: series-by rows ($sb)" || { echo "  FAIL: series-by returned 0 rows"; fail=1; }

# series: >=1 bucket; latest-snapshot total cpu = 250+10+30+100 = 390
buckets=$(psql -c "SELECT count(*) FROM (SELECT time_bucket('1 minute', time) t, avg(scpu) cpu FROM \
  (SELECT time, sum(cpu_millicores) scpu FROM kube_container_stats WHERE system_id='$SID' GROUP BY time) s GROUP BY 1) x;")
[ "$buckets" -ge 1 ] && echo "  ok: series buckets ($buckets)" || { echo "  FAIL: series buckets 0"; fail=1; }
scpu=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats WHERE system_id='$SID' \
  AND time=(SELECT max(time) FROM kube_container_stats WHERE system_id='$SID');")
assert "latest snapshot total cpu" "$scpu" "390"

# /api/kube/summaries — the per-cluster LATERAL roll-up behind the Clusters page.
read -r as_of cpu mem pods run conts rst ns nodes <<<"$(psql -c "
  SELECT agg.* FROM unnest(ARRAY['$SID']::uuid[]) AS s(sid)
  CROSS JOIN LATERAL (
    SELECT extract(epoch FROM max(c.time))::int8 AS as_of,
           COALESCE(sum(cpu_millicores),0)::float8, COALESCE(sum(mem_bytes),0)::float8,
           count(DISTINCT pod)::int8, count(DISTINCT pod) FILTER (WHERE phase='Running')::int8,
           count(*)::int8, COALESCE(sum(restarts),0)::int8,
           count(DISTINCT namespace)::int8, count(DISTINCT node) FILTER (WHERE node <> '')::int8
    FROM kube_container_stats c WHERE c.system_id = s.sid
      AND c.time > now() - interval '$FRESH'
      AND c.time = (SELECT max(time) FROM kube_container_stats
                    WHERE system_id = s.sid AND time > now() - interval '$FRESH')
  ) agg WHERE agg.as_of IS NOT NULL;" | tr '|' ' ')"
assert "summaries cpu" "$cpu" "390"
assert "summaries mem" "$mem" "185597952"
assert "summaries pods" "$pods" "3"
assert "summaries pods_running" "$run" "3"
assert "summaries containers" "$conts" "4"
assert "summaries restarts" "$rst" "3"
assert "summaries namespaces" "$ns" "3"
assert "summaries nodes" "$nodes" "3"
[ -n "$as_of" ] && echo "  ok: summaries as_of set" || { echo "  FAIL: summaries as_of null"; fail=1; }

# Chunk exclusion: an old snapshot must land in its own chunk that the bounded
# "latest" query never reads. The unbounded shape (the pre-fix one) reads both —
# that is what made /clusters hammer Postgres.
docker exec -i "$CID" psql -v ON_ERROR_STOP=1 -q -U vantage -d vantage_data >/dev/null <<SQL
INSERT INTO kube_container_stats
  (time, system_id, namespace, pod, container, node, phase, workload, workload_kind, cpu_millicores, mem_bytes, restarts, labels)
VALUES (now() - interval '10 days', '$SID'::uuid, 'default','old-1','app','n1','Running','web','Deployment',1,1,0,'{}'::jsonb);
SQL
chunks() { psql -c "EXPLAIN (COSTS OFF) $1" | grep -c '_hyper_[0-9]*_[0-9]*_chunk' || true; }
new=$(chunks "SELECT sum(cpu_millicores) FROM kube_container_stats c WHERE $W;")
old=$(chunks "WITH l AS (SELECT max(time) t FROM kube_container_stats WHERE system_id='$SID') \
  SELECT sum(cpu_millicores) FROM kube_container_stats c, l WHERE c.system_id='$SID' AND c.time=l.t;")
if [ "$new" -lt "$old" ]; then echo "  ok: chunk exclusion ($new chunk scans vs $old unbounded)"
else echo "  FAIL: bounded query scans $new chunks, unbounded $old — no exclusion"; fail=1; fi
# ...and it must still be correct with the old row present.
scpu2=$(psql -c "SELECT sum(cpu_millicores) FROM kube_container_stats c WHERE $W;")
assert "latest snapshot cpu ignores old chunk" "$scpu2" "390"

[ "$fail" = 0 ] && echo "PASS — kube_container_stats migration + ingest + read queries valid" || { echo "FAILED"; exit 1; }
