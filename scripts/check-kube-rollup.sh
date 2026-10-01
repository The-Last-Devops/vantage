#!/usr/bin/env bash
# Prove the k8s rollup ladder stores what the charts actually want, against a throwaway
# TimescaleDB. Idempotent + self-cleaning.
#   bash scripts/check-kube-rollup.sh
#
# The thing being guarded is an easy mistake with a silent, plausible-looking failure:
# a chart wants, per bucket, the AVERAGE ACROSS SNAPSHOTS OF THE SUM ACROSS CONTAINERS.
# Writing the obvious `avg(cpu_millicores)` instead gives the average of one container,
# which still draws a smooth believable line — just one that is wrong by a factor of
# however many containers are running. So this asserts the exact expected number AND
# asserts it differs from the naive one.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CID="vantage-rolluptest-$$"
SID="00000000-0000-0000-0000-000000000001"
cleanup() { docker rm -f "$CID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

q() { docker exec "$CID" psql -tAqX -U vantage -d vantage_data -c "$1"; }

echo "starting throwaway TimescaleDB…"
docker run -d --name "$CID" \
  -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage -e POSTGRES_DB=vantage_data \
  timescale/timescaledb:2.17.2-pg16 >/dev/null

echo "waiting for readiness…"
ok=0
for i in $(seq 1 90); do
  if q "SELECT 1" >/dev/null 2>&1; then ok=$((ok + 1)); [ "$ok" -ge 4 ] && break; else ok=0; fi
  sleep 1
done
[ "$ok" -ge 4 ] || { echo "FAIL: database never became ready"; exit 1; }

echo "applying data migrations…"
for f in "$REPO"/migrations/data/*.sql; do
  docker exec -i "$CID" psql -qX -U vantage -d vantage_data < "$f" >/dev/null
done

# The two write-only tables must be gone after 0003 — if a later migration or a merge
# resurrects them, the 5.6 GB that caused three weeks of data loss comes back with it.
for dead in kube_namespace_stats kube_deployment_stats; do
  got=$(q "SELECT to_regclass('public.$dead') IS NULL")
  [ "$got" = "t" ] || { echo "FAIL: $dead still exists — it has no reader and must stay dropped"; exit 1; }
done
echo "  ✓ unread kube tables are gone"

# Two snapshots inside ONE 5-minute bucket, two containers in the same group.
#   snapshot 1: 100 + 200 = 300 millicores
#   snapshot 2: 400 + 600 = 1000 millicores
# → cpu_avg = (300 + 1000) / 2 = 650, cpu_max = 1000, containers = 2
# The naive per-row average would be (100+200+400+600)/4 = 325.
echo "seeding two snapshots…"
q "INSERT INTO kube_container_stats
     (time, system_id, namespace, pod, container, node, phase, workload, workload_kind,
      cpu_millicores, mem_bytes, restarts, labels)
   VALUES
     ('2026-01-01 00:00:00+00','$SID','prod','web-a','c','n1','Running','web','Deployment',100,1000,0,'{}'),
     ('2026-01-01 00:00:00+00','$SID','prod','web-b','c','n1','Running','web','Deployment',200,2000,0,'{}'),
     ('2026-01-01 00:01:00+00','$SID','prod','web-a','c','n1','Running','web','Deployment',400,4000,1,'{}'),
     ('2026-01-01 00:01:00+00','$SID','prod','web-b','c','n1','Running','web','Deployment',600,6000,0,'{}');" >/dev/null

# Mirrors fill_5m() in crates/hub/src/kube_rollup.rs. Keep the two in step: the shape
# (inner GROUP BY including `time`, outer aggregating those per-snapshot totals) is the
# whole point of the test.
FILL_5M="INSERT INTO kube_rollup_5m
   (bucket, system_id, namespace, workload, workload_kind, node,
    cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts)
 SELECT b, system_id, namespace, workload, workload_kind, node,
        avg(cpu), max(cpu), avg(mem), max(mem), avg(n), max(restarts)
 FROM (
   SELECT time_bucket(INTERVAL '5 minutes', time) AS b, time, system_id,
          namespace, workload, workload_kind, node,
          sum(cpu_millicores)::float8 AS cpu, sum(mem_bytes)::float8 AS mem,
          count(*)::float8 AS n, max(restarts) AS restarts
   FROM kube_container_stats
   GROUP BY 1,2,3,4,5,6,7
 ) s GROUP BY 1,2,3,4,5,6
 ON CONFLICT DO NOTHING;"

q "$FILL_5M" >/dev/null
got=$(q "SELECT cpu_avg || '/' || cpu_max || '/' || mem_avg || '/' || containers FROM kube_rollup_5m")
[ "$got" = "650/1000/6500/2" ] || { echo "FAIL: 5m rollup = $got, expected 650/1000/6500/2"; exit 1; }
echo "  ✓ 5m tier stores avg-across-snapshots of sum-across-containers (650, not 325)"

# Idempotency: the job re-runs the same window after a restart or a retry.
q "$FILL_5M" >/dev/null
n=$(q "SELECT count(*) FROM kube_rollup_5m")
[ "$n" = "1" ] || { echo "FAIL: refill duplicated rows (count=$n) — the unique key is not holding"; exit 1; }
echo "  ✓ refilling the same window is idempotent"

# 1h tier composes from 5m: averages of averages, max of maxes (so a peak survives).
q "INSERT INTO kube_rollup_1h
     (bucket, system_id, namespace, workload, workload_kind, node,
      cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts)
   SELECT time_bucket(INTERVAL '1 hour', bucket), system_id, namespace, workload,
          workload_kind, node,
          avg(cpu_avg), max(cpu_max), avg(mem_avg), max(mem_max),
          avg(containers), max(restarts)
   FROM kube_rollup_5m GROUP BY 1,2,3,4,5,6
   ON CONFLICT DO NOTHING;" >/dev/null
got=$(q "SELECT cpu_avg || '/' || cpu_max FROM kube_rollup_1h")
[ "$got" = "650/1000" ] || { echo "FAIL: 1h rollup = $got, expected 650/1000"; exit 1; }
echo "  ✓ 1h tier keeps the peak snapshot, not a flattened average"

# The hub must never answer a long range from the raw tier again — that is what made
# /kube/series scan tens of millions of rows and time out on the busiest cluster.
grep -q 'Some("12h") | Some("24h") | Some("7d") => T5M' "$REPO/crates/hub/src/web/kube.rs" \
  || { echo "FAIL: kube_tier no longer routes 12h/24h/7d to the 5m rollup"; exit 1; }
grep -q 'Some("30d") | Some("90d") | Some("1y") => T1H' "$REPO/crates/hub/src/web/kube.rs" \
  || { echo "FAIL: kube_tier no longer routes long ranges to the 1h rollup"; exit 1; }
echo "  ✓ read path routes 12h/24h/7d → 5m and 30d/90d/1y → 1h"

# Eviction must not be free to eat the long tiers while cheap detail is still droppable.
grep -q '"system_metrics_1h", "kube_rollup_1h", "heartbeats"' "$REPO/crates/hub/src/data_admin.rs" \
  || { echo "FAIL: PROTECTED_TIERS no longer covers the long-horizon tiers"; exit 1; }

# …and prove the SELECTION actually honours it, not just that the list exists. Make the
# PROTECTED table the biggest one, which is precisely the case plain largest-tier-first
# gets wrong: a year of hourly rollup does eventually outgrow two days of raw.
echo "seeding a database where the protected tier is the largest…"
q "INSERT INTO kube_rollup_1h (bucket, system_id, namespace, workload, workload_kind, node,
      cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts)
   SELECT g, '$SID', 'ns' || (i % 50), 'w' || i, 'Deployment', 'n1', 1,1,1,1,1,0
   FROM generate_series('2026-01-01'::timestamptz, '2026-02-10'::timestamptz, INTERVAL '1 hour') g,
        generate_series(1, 40) i
   ON CONFLICT DO NOTHING;" >/dev/null
q "INSERT INTO kube_container_stats
     (time, system_id, namespace, pod, container, node, phase, workload, workload_kind,
      cpu_millicores, mem_bytes, restarts, labels)
   SELECT g, '$SID', 'prod', 'p' || i, 'c', 'n1', 'Running', 'web', 'Deployment', 1, 1, 0, '{}'
   FROM generate_series('2026-01-01'::timestamptz, '2026-01-03'::timestamptz, INTERVAL '5 minutes') g,
        generate_series(1, 5) i;" >/dev/null

# Same query as evict_target() in data_admin.rs.
TARGET="SELECT c.hypertable_name
        FROM timescaledb_information.hypertables c
        WHERE c.hypertable_schema = 'public' AND c.hypertable_name <> ALL(%s)
        ORDER BY hypertable_size(format('%%I.%%I', c.hypertable_schema, c.hypertable_name)::regclass) DESC NULLS LAST
        LIMIT 1;"

biggest=$(q "$(printf "$TARGET" "ARRAY[]::text[]")")
[ "$biggest" = "kube_rollup_1h" ] || { echo "FAIL: test setup wrong — biggest table is $biggest, expected kube_rollup_1h"; exit 1; }
picked=$(q "$(printf "$TARGET" "ARRAY['system_metrics_1h','kube_rollup_1h','heartbeats']::text[]")")
[ "$picked" = "kube_container_stats" ] || { echo "FAIL: eviction picked $picked — it must skip the protected tier while raw still has chunks"; exit 1; }
echo "  ✓ eviction skips the protected tier even when it is the LARGEST table"

echo "OK — kube rollup ladder behaves as specified."
