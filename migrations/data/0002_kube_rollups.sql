-- K8s metrics: give them the rollup ladder that host metrics already had.
--
-- Why: `kube_container_stats` is the ONLY k8s table any read path touches, it was
-- ingested every 15s with no downsampling, and it grew ~4.7 GB/day. The data cap then
-- evicted one day-chunk of it every single day for three weeks, so its declared 14-day
-- retention was really ~2.3 days and every cluster chart cut off at the same chunk
-- boundary. Rolling up is what makes a long lookback affordable at all.
--
-- The rollups are PLAIN TABLES filled by a job, NOT continuous aggregates, on purpose:
-- a CAgg cannot be ALTERed, so adding a metric column forces a drop + rebuild of the
-- whole chain, and the long tier can then only refill from whatever the shorter tier
-- still holds. That is exactly why `system_metrics_1h` held ~44 days of data despite a
-- 365-day policy. A real table takes ADD COLUMN and keeps its history.

-- Rolled up per (cluster, namespace, workload, node) — NOT per pod/container. Pod names
-- change on every rollout, so keeping them is what made the raw table unaffordable; the
-- charts only ever group by namespace / workload / node anyway. `labels` is dropped for
-- the same reason, so label filtering stays on the raw tier (short ranges only).
--
-- cpu_avg/mem_avg are the average ACROSS SNAPSHOTS of the per-snapshot sum, which is
-- what the charts draw. cpu_max/mem_max keep the peak snapshot so a long-range lookback
-- can still show a spike that an hourly average would otherwise flatten away.
CREATE TABLE kube_rollup_5m (
    bucket        TIMESTAMPTZ NOT NULL,
    system_id     UUID NOT NULL,
    namespace     TEXT NOT NULL,
    workload      TEXT NOT NULL,
    workload_kind TEXT NOT NULL,
    node          TEXT NOT NULL,
    cpu_avg       DOUBLE PRECISION NOT NULL,
    cpu_max       DOUBLE PRECISION NOT NULL,
    mem_avg       DOUBLE PRECISION NOT NULL,
    mem_max       DOUBLE PRECISION NOT NULL,
    containers    DOUBLE PRECISION NOT NULL,
    restarts      INTEGER NOT NULL DEFAULT 0
);
SELECT create_hypertable('kube_rollup_5m', 'bucket', chunk_time_interval => INTERVAL '1 day');
-- The unique key is what makes the fill job idempotent (ON CONFLICT DO NOTHING); a
-- hypertable's unique index must include the partitioning column, hence bucket first.
CREATE UNIQUE INDEX idx_kube_5m_key
    ON kube_rollup_5m (bucket, system_id, namespace, workload, workload_kind, node);
CREATE INDEX idx_kube_5m_sys_bucket ON kube_rollup_5m (system_id, bucket DESC);

CREATE TABLE kube_rollup_1h (
    bucket        TIMESTAMPTZ NOT NULL,
    system_id     UUID NOT NULL,
    namespace     TEXT NOT NULL,
    workload      TEXT NOT NULL,
    workload_kind TEXT NOT NULL,
    node          TEXT NOT NULL,
    cpu_avg       DOUBLE PRECISION NOT NULL,
    cpu_max       DOUBLE PRECISION NOT NULL,
    mem_avg       DOUBLE PRECISION NOT NULL,
    mem_max       DOUBLE PRECISION NOT NULL,
    containers    DOUBLE PRECISION NOT NULL,
    restarts      INTEGER NOT NULL DEFAULT 0
);
SELECT create_hypertable('kube_rollup_1h', 'bucket', chunk_time_interval => INTERVAL '7 days');
CREATE UNIQUE INDEX idx_kube_1h_key
    ON kube_rollup_1h (bucket, system_id, namespace, workload, workload_kind, node);
CREATE INDEX idx_kube_1h_sys_bucket ON kube_rollup_1h (system_id, bucket DESC);

-- How far each tier has been filled, so the job resumes where it stopped instead of
-- rescanning — and so a restart cannot silently leave a hole in the middle.
CREATE TABLE kube_rollup_progress (
    tier      TEXT PRIMARY KEY,
    filled_to TIMESTAMPTZ NOT NULL
);
