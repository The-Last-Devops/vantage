#!/usr/bin/env bash
# Prove the three-tier ladder (raw -> 1m -> 1h) against a throwaway TimescaleDB.
# Idempotent + self-cleaning.
#   bash scripts/check-rollup.sh
#
# The headline thing being guarded is an easy mistake with a silent, plausible-looking
# failure: for k8s a chart wants, per bucket, the AVERAGE ACROSS SNAPSHOTS OF THE SUM
# ACROSS CONTAINERS. Writing the obvious `avg(cpu_millicores)` instead gives the average
# of one container — still a smooth, believable line, just wrong by however many
# containers are running. So this asserts the exact expected number AND asserts it
# differs from the naive one.
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

# --- shape -----------------------------------------------------------------------
# Exactly these metric tables, and every one of them names its resolution. An
# unsuffixed `system_metrics` next to `system_metrics_1m` cannot be read correctly:
# nothing tells you which is finer.
want="container_metrics_5s kube_metrics_1h kube_metrics_1m system_metrics_1h system_metrics_1m system_metrics_5s"
got=$(q "SELECT string_agg(hypertable_name, ' ' ORDER BY hypertable_name)
         FROM timescaledb_information.hypertables
         WHERE hypertable_schema = 'public' AND hypertable_name <> 'heartbeats';")
[ "$got" = "$want" ] || { echo "FAIL: tier tables are"; echo "  $got"; echo "expected"; echo "  $want"; exit 1; }
echo "  ✓ every metric tier names its cadence"

# Tables that must stay gone: two that were written and never read, and the middle rungs
# of the old ladders.
for dead in kube_namespace_stats kube_deployment_stats kube_rollup_5m kube_container_stats \
            system_metrics system_metrics_5m system_metrics_15m container_metrics; do
  [ "$(q "SELECT to_regclass('public.$dead') IS NULL")" = "t" ] \
    || { echo "FAIL: $dead still exists"; exit 1; }
done
echo "  ✓ removed tables and old rung names stay removed"

# The rollups must be ordinary tables. If one is a continuous aggregate again, adding a
# metric column will silently reset its history — the bug that left the hourly host tier
# holding 43 days under a 365-day policy.
cagg=$(q "SELECT count(*) FROM timescaledb_information.continuous_aggregates;")
[ "$cagg" = "0" ] || { echo "FAIL: $cagg continuous aggregate(s) exist — rollups must be plain tables"; exit 1; }
echo "  ✓ no continuous aggregates (rollups survive ADD COLUMN)"

# The migration must copy the old hourly rollup out BEFORE dropping the chain that
# CASCADEs into it. Getting this order wrong silently destroys the longest history there.
mig="$REPO/migrations/data/0004_three_tier_metrics.sql"
copy_at=$(grep -n 'FROM system_metrics_1h;' "$mig" | head -1 | cut -d: -f1)
drop_at=$(grep -n 'DROP MATERIALIZED VIEW IF EXISTS system_metrics_1h' "$mig" | head -1 | cut -d: -f1)
[ -n "$copy_at" ] && [ -n "$drop_at" ] && [ "$copy_at" -lt "$drop_at" ] \
  || { echo "FAIL: migration drops the hourly view at line $drop_at before copying it at $copy_at"; exit 1; }
echo "  ✓ migration copies the hourly history before dropping the old chain"

# --- k8s aggregation -------------------------------------------------------------
# Two snapshots inside ONE hour, two containers in the same group.
#   snapshot 1: 100 + 200 = 300 millicores
#   snapshot 2: 400 + 600 = 1000 millicores
# → cpu_avg = (300 + 1000) / 2 = 650, cpu_max = 1000, containers = 2
# The naive per-row average would be (100+200+400+600)/4 = 325.
echo "seeding two k8s snapshots…"
q "INSERT INTO kube_metrics_1m
     (time, system_id, namespace, pod, container, node, phase, workload, workload_kind,
      cpu_millicores, mem_bytes, restarts, labels)
   VALUES
     ('2026-01-01 00:00:00+00','$SID','prod','web-a','c','n1','Running','web','Deployment',100,1000,0,'{}'),
     ('2026-01-01 00:00:00+00','$SID','prod','web-b','c','n1','Running','web','Deployment',200,2000,0,'{}'),
     ('2026-01-01 00:10:00+00','$SID','prod','web-a','c','n1','Running','web','Deployment',400,4000,1,'{}'),
     ('2026-01-01 00:10:00+00','$SID','prod','web-b','c','n1','Running','web','Deployment',600,6000,0,'{}');" >/dev/null

# Mirrors fill_kube() in crates/hub/src/rollup.rs. Keep the two in step: the shape (inner
# GROUP BY including the sample time, outer aggregating those per-snapshot totals) is the
# whole point of the test.
FILL_KUBE="INSERT INTO kube_metrics_1h
   (bucket, system_id, namespace, workload, workload_kind, node,
    cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts)
 SELECT b, system_id, namespace, workload, workload_kind, node,
        avg(cpu), max(cpu), avg(mem), max(mem), avg(n), max(restarts)
 FROM (
   SELECT time_bucket(INTERVAL '1 hour', time) AS b, time, system_id,
          namespace, workload, workload_kind, node,
          sum(cpu_millicores)::float8 AS cpu, sum(mem_bytes)::float8 AS mem,
          count(*)::float8 AS n, max(restarts) AS restarts
   FROM kube_metrics_1m GROUP BY 1,2,3,4,5,6,7
 ) s GROUP BY 1,2,3,4,5,6
 ON CONFLICT DO NOTHING;"

q "$FILL_KUBE" >/dev/null
got=$(q "SELECT cpu_avg || '/' || cpu_max || '/' || mem_avg || '/' || containers FROM kube_metrics_1h")
[ "$got" = "650/1000/6500/2" ] || { echo "FAIL: k8s 1h rollup = $got, expected 650/1000/6500/2"; exit 1; }
echo "  ✓ k8s 1h stores avg-across-snapshots of sum-across-containers (650, not 325)"

q "$FILL_KUBE" >/dev/null
n=$(q "SELECT count(*) FROM kube_metrics_1h")
[ "$n" = "1" ] || { echo "FAIL: refill duplicated rows (count=$n) — the unique key is not holding"; exit 1; }
echo "  ✓ refilling the same window is idempotent"

# --- host aggregation ------------------------------------------------------------
# One host, two raw samples in the same minute. cpu 10 and 30 -> avg 20; net_rx is a
# cumulative counter, so 500 then 900 -> max 900, which is what survives further rollup.
q "INSERT INTO system_metrics_5s
     (time, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
      disk_used, disk_total, net_rx, net_tx, load1, uptime)
   VALUES
     ('2026-01-01 00:00:00+00','$SID',10,1,2,0,0,1,2,500,100,0.5,60),
     ('2026-01-01 00:00:30+00','$SID',30,3,2,0,0,3,2,900,300,1.5,90);" >/dev/null
q "INSERT INTO system_metrics_1m (bucket, system_id, cpu_percent, net_rx)
   SELECT time_bucket(INTERVAL '1 minute', time), system_id, avg(cpu_percent)::float8, max(net_rx)
   FROM system_metrics_5s GROUP BY 1,2 ON CONFLICT DO NOTHING;" >/dev/null
got=$(q "SELECT cpu_percent || '/' || net_rx FROM system_metrics_1m")
[ "$got" = "20/900" ] || { echo "FAIL: host 1m rollup = $got, expected 20/900 (avg cpu, max counter)"; exit 1; }
echo "  ✓ host 1m averages gauges and takes the max of counters"

# --- read routing ----------------------------------------------------------------
grep -q 'Some("7d") | Some("30d") | Some("90d") | Some("1y") => T1H' "$REPO/crates/hub/src/web/kube.rs" \
  || { echo "FAIL: kube_tier no longer routes long ranges to the hourly tier"; exit 1; }
grep -q 'Some("24h") => ("_1m", "bucket"' "$REPO/crates/hub/src/web/mod.rs" \
  || { echo "FAIL: chart_tier no longer routes 24h to the 1-minute tier"; exit 1; }
grep -q 'Some("7d") => ("_1h", "bucket"' "$REPO/crates/hub/src/web/mod.rs" \
  || { echo "FAIL: chart_tier no longer routes 7d to the hourly tier"; exit 1; }
echo "  ✓ read path routes each range to the coarsest tier that still divides its bucket"

# Eviction must not be free to eat the long tiers while cheap detail is still droppable.
grep -q '"system_metrics_1h", "kube_metrics_1h", "heartbeats"' "$REPO/crates/hub/src/data_admin.rs" \
  || { echo "FAIL: PROTECTED_TIERS no longer covers the long-horizon tiers"; exit 1; }

# …and prove the SELECTION honours it, not just that the list exists. Make the PROTECTED
# table the biggest one — precisely the case plain largest-tier-first gets wrong.
q "INSERT INTO kube_metrics_1h (bucket, system_id, namespace, workload, workload_kind, node,
      cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts)
   SELECT g, '$SID', 'ns' || (i % 50), 'w' || i, 'Deployment', 'n1', 1,1,1,1,1,0
   FROM generate_series('2026-02-01'::timestamptz, '2026-03-10'::timestamptz, INTERVAL '1 hour') g,
        generate_series(1, 40) i
   ON CONFLICT DO NOTHING;" >/dev/null
q "INSERT INTO kube_metrics_1m
     (time, system_id, namespace, pod, container, node, phase, workload, workload_kind,
      cpu_millicores, mem_bytes, restarts, labels)
   SELECT g, '$SID', 'prod', 'p' || i, 'c', 'n1', 'Running', 'web', 'Deployment', 1, 1, 0, '{}'
   FROM generate_series('2026-02-01'::timestamptz, '2026-02-03'::timestamptz, INTERVAL '5 minutes') g,
        generate_series(1, 5) i;" >/dev/null

TARGET="SELECT c.hypertable_name
        FROM timescaledb_information.hypertables c
        WHERE c.hypertable_schema = 'public' AND c.hypertable_name <> ALL(%s)
        ORDER BY hypertable_size(format('%%I.%%I', c.hypertable_schema, c.hypertable_name)::regclass) DESC NULLS LAST
        LIMIT 1;"
biggest=$(q "$(printf "$TARGET" "ARRAY[]::text[]")")
[ "$biggest" = "kube_metrics_1h" ] || { echo "FAIL: test setup wrong — biggest is $biggest"; exit 1; }
picked=$(q "$(printf "$TARGET" "ARRAY['system_metrics_1h','kube_metrics_1h','heartbeats']::text[]")")
[ "$picked" = "kube_metrics_1m" ] || { echo "FAIL: eviction picked $picked — it must skip the protected tier"; exit 1; }
echo "  ✓ eviction skips the protected tier even when it is the LARGEST table"

echo "OK — three-tier ladder behaves as specified."
