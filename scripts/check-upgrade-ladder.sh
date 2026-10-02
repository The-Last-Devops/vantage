#!/usr/bin/env bash
# Prove the 3.3.0 migration preserves history when upgrading a REAL installation.
#   bash scripts/check-upgrade-ladder.sh
#
# check-rollup.sh exercises a fresh database, where the old rollup views never existed.
# Production is the opposite case and the dangerous one: `system_metrics_1h` is a
# continuous aggregate built on _15m on _5m on _1m, so dropping any of them CASCADEs
# into it. The hourly tier is the longest history the hub has — on the live hub, 43 days
# of it — and it cannot be rebuilt from anything, because its sources are kept 45 days at
# most. If 0004 drops before it copies, that history is simply gone, silently, during a
# routine `helm upgrade`.
#
# So: build the PRE-3.3.0 state exactly as the old startup code did, put recognisable
# data in it, run the migration, and assert the rows came out the other side.
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CID="vantage-upgradetest-$$"
SID="00000000-0000-0000-0000-000000000001"
cleanup() { docker rm -f "$CID" >/dev/null 2>&1 || true; }
trap cleanup EXIT

q() { docker exec "$CID" psql -tAqX -U vantage -d vantage_data -c "$1"; }

echo "starting throwaway TimescaleDB…"
docker run -d --name "$CID" \
  -e POSTGRES_USER=vantage -e POSTGRES_PASSWORD=vantage -e POSTGRES_DB=vantage_data \
  timescale/timescaledb:2.17.2-pg16 >/dev/null
ok=0
for i in $(seq 1 90); do
  if q "SELECT 1" >/dev/null 2>&1; then ok=$((ok + 1)); [ "$ok" -ge 4 ] && break; else ok=0; fi
  sleep 1
done
[ "$ok" -ge 4 ] || { echo "FAIL: database never became ready"; exit 1; }

echo "applying migrations up to 0003 (the 3.2.x schema)…"
for f in "$REPO"/migrations/data/0001_init.sql "$REPO"/migrations/data/0002_*.sql "$REPO"/migrations/data/0003_*.sql; do
  docker exec -i "$CID" psql -qX -U vantage -d vantage_data < "$f" >/dev/null
done

echo "seeding raw host samples spanning 3 hours…"
q "INSERT INTO system_metrics
     (time, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
      disk_used, disk_total, net_rx, net_tx, load1, uptime)
   SELECT g, '$SID', 42, 100, 200, 0, 0, 10, 20, 1000, 2000, 1.0, 3600
   FROM generate_series(now() - interval '3 hours', now() - interval '10 minutes', INTERVAL '5 seconds') g;" >/dev/null

# Rebuild the old ladder the way data_admin::setup did before 3.3.0: four chained
# continuous aggregates, the hourly one fed from the 15-minute one.
echo "building the pre-3.3.0 continuous-aggregate ladder…"
AGG="avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, avg(mem_total) AS mem_total, \
avg(swap_used) AS swap_used, avg(swap_total) AS swap_total, avg(disk_used) AS disk_used, \
avg(disk_total) AS disk_total, max(net_rx) AS net_rx, max(net_tx) AS net_tx, \
max(disk_read) AS disk_read, max(disk_write) AS disk_write, avg(load1) AS load1, \
avg(load5) AS load5, avg(load15) AS load15, avg(cpu_user) AS cpu_user, \
avg(cpu_system) AS cpu_system, avg(cpu_iowait) AS cpu_iowait, avg(cpu_steal) AS cpu_steal, \
avg(disk_util) AS disk_util, avg(mem_available) AS mem_available, \
avg(mem_buffers) AS mem_buffers, avg(mem_cached) AS mem_cached, avg(mem_free) AS mem_free"
mk() { # name bucket source srccol
  q "CREATE MATERIALIZED VIEW $1 WITH (timescaledb.continuous) AS
     SELECT system_id, time_bucket('$2', $4) AS bucket, $AGG
     FROM $3 GROUP BY system_id, time_bucket('$2', $4) WITH NO DATA;" >/dev/null
  q "CALL refresh_continuous_aggregate('$1', NULL, NULL);" >/dev/null
}
mk system_metrics_1m  "1 minute"   system_metrics      time
mk system_metrics_5m  "5 minutes"  system_metrics_1m   bucket
mk system_metrics_15m "15 minutes" system_metrics_5m   bucket
mk system_metrics_1h  "1 hour"     system_metrics_15m  bucket

before_1h=$(q "SELECT count(*) FROM system_metrics_1h")
before_1m=$(q "SELECT count(*) FROM system_metrics_1m")
before_cpu=$(q "SELECT round(avg(cpu_percent)::numeric, 3) FROM system_metrics_1h")
[ "$before_1h" -gt 0 ] || { echo "FAIL: test setup produced no hourly rows"; exit 1; }
echo "  pre-upgrade: ${before_1h} hourly rows, ${before_1m} minute rows, avg cpu ${before_cpu}"

# A k8s row too, so the rename path is covered on data that exists.
q "INSERT INTO kube_container_stats
     (time, system_id, namespace, pod, container, node, phase, workload, workload_kind,
      cpu_millicores, mem_bytes, restarts, labels)
   VALUES (now() - interval '1 hour','$SID','prod','p','c','n1','Running','web','Deployment',7,70,3,'{}');" >/dev/null

echo "applying 0004…"
docker exec -i "$CID" psql -qX -U vantage -d vantage_data < "$REPO"/migrations/data/0004_*.sql >/dev/null

after_1h=$(q "SELECT count(*) FROM system_metrics_1h")
after_1m=$(q "SELECT count(*) FROM system_metrics_1m")
after_cpu=$(q "SELECT round(avg(cpu_percent)::numeric, 3) FROM system_metrics_1h")

[ "$after_1h" = "$before_1h" ] \
  || { echo "FAIL: hourly history lost — $before_1h rows before, $after_1h after"; exit 1; }
[ "$after_1m" = "$before_1m" ] \
  || { echo "FAIL: minute history lost — $before_1m rows before, $after_1m after"; exit 1; }
[ "$after_cpu" = "$before_cpu" ] \
  || { echo "FAIL: hourly VALUES changed — $before_cpu before, $after_cpu after"; exit 1; }
echo "  ✓ hourly and minute history survive the migration, values unchanged"

# They must now be plain tables: that is the whole point, so a future metric column is an
# ADD COLUMN instead of a rebuild that resets history to whatever the tier below holds.
[ "$(q "SELECT count(*) FROM timescaledb_information.continuous_aggregates")" = "0" ] \
  || { echo "FAIL: a continuous aggregate survived the migration"; exit 1; }
q "ALTER TABLE system_metrics_1h ADD COLUMN probe double precision;" >/dev/null
[ "$(q "SELECT count(*) FROM system_metrics_1h")" = "$before_1h" ] \
  || { echo "FAIL: ADD COLUMN disturbed the hourly tier"; exit 1; }
q "ALTER TABLE system_metrics_1h DROP COLUMN probe;" >/dev/null
echo "  ✓ hourly tier is a plain table and takes ADD COLUMN without losing rows"

# Renames carried the k8s data across rather than recreating empty tables.
[ "$(q "SELECT restarts FROM kube_metrics_1m")" = "3" ] \
  || { echo "FAIL: kube detail rows did not survive the rename"; exit 1; }
[ "$(q "SELECT count(*) FROM system_metrics_5s")" -gt 0 ] \
  || { echo "FAIL: raw host rows did not survive the rename"; exit 1; }
echo "  ✓ renames preserved k8s detail and raw host rows"

echo "OK — upgrading a populated 3.2.x database keeps its history."
