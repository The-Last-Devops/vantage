#!/usr/bin/env bash
# Diagnose "old rows are not being deleted": prints every TimescaleDB retention /
# refresh / compression job on the DATA DB together with its last run outcome, plus
# the background-worker settings that silently disable ALL of them when set to 0.
#
# Usage:  DATA_DATABASE_URL=postgres://user:pass@host/vantage_data bash scripts/check-retention-jobs.sh
#   (in-cluster:  kubectl exec -n <ns> deploy/vantage-hub -- env | grep DATA_DATABASE_URL)
set -euo pipefail

: "${DATA_DATABASE_URL:?set DATA_DATABASE_URL to the data DB connection string}"
psql() { command psql "$DATA_DATABASE_URL" -X -q "$@"; }

echo "== background workers (if max_background_workers = 0, NOTHING ever runs) =="
psql -c "SELECT name, setting FROM pg_settings
         WHERE name IN ('timescaledb.max_background_workers','max_worker_processes');"

echo "== jobs + last run =="
psql -c "SELECT j.job_id, j.proc_name, j.hypertable_name, j.schedule_interval,
                j.config->>'drop_after' AS drop_after, j.scheduled,
                s.last_run_started_at, s.last_successful_finish, s.last_run_status,
                s.total_failures
         FROM timescaledb_information.jobs j
         LEFT JOIN timescaledb_information.job_stats s USING (job_id)
         ORDER BY j.proc_name, j.job_id;"

echo "== oldest chunk per hypertable (should not predate its drop_after) =="
psql -c "SELECT hypertable_name, count(*) AS chunks, min(range_start) AS oldest, max(range_end) AS newest
         FROM timescaledb_information.chunks
         GROUP BY 1 ORDER BY 1;"

echo "== rollup views -> materialization hypertable (retention is recorded on the latter) =="
psql -c "SELECT view_name, materialization_hypertable_name FROM timescaledb_information.continuous_aggregates ORDER BY 1;"

echo "OK — read the job rows above: last_run_status must be 'Success' and total_failures 0."
