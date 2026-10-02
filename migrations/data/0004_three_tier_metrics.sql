-- One ladder shape for every metric family: raw -> 1m -> 1h.
--
-- Host had five rungs (5s/1m/5m/15m/1h) and k8s three (60s/5m/1h), with different
-- names, different mechanisms and different retention logic. The middle rungs earned
-- very little: every chart range is served by raw, 1m or 1h, because the display
-- buckets the UI actually asks for are 1m, 2m, 5m, 10m, 15m, 1h, 6h and 1d — and a
-- tier serves any bucket that is a multiple of it.
--
-- K8s has no sub-minute rung on purpose. Its samples come from metrics-server, which
-- defaults to a 60s resolution and documents 15s as the floor because that is what
-- kubelet itself computes. Scraping faster returns the same number repeatedly, so the
-- 60s detail table IS the 1-minute tier — hence `kube_metrics_1m` keeps pod and label
-- detail, and is both the drill-down source and the short-range chart source.
--
-- The host rollups stop being continuous aggregates here. A CAgg cannot be ALTERed, so
-- adding one metric column forces a drop and rebuild of the whole chain, and the long
-- tier can then only refill from the tier below it — which is why `system_metrics_1h`
-- held 43 days under a 365-day policy. Plain tables take ADD COLUMN and keep history.
--
-- ORDER MATTERS BELOW: the 1h view is built on 15m on 5m on 1m, so dropping any of them
-- CASCADEs into 1h. Its 43 days are copied out FIRST; getting this wrong destroys the
-- exact history this migration exists to protect.

-- ---------------------------------------------------------------- host: 1m and 1h
-- Column types mirror what the aggregates produced and what the read paths consume:
-- the counters are max() of BIGINT and must stay BIGINT (metrics.rs decodes them as
-- i64); everything else is an average and is stored as float8.
CREATE TABLE system_metrics_1m_new (
    bucket        TIMESTAMPTZ NOT NULL,
    system_id     UUID NOT NULL,
    cpu_percent   DOUBLE PRECISION,
    mem_used      DOUBLE PRECISION,
    mem_total     DOUBLE PRECISION,
    swap_used     DOUBLE PRECISION,
    swap_total    DOUBLE PRECISION,
    disk_used     DOUBLE PRECISION,
    disk_total    DOUBLE PRECISION,
    net_rx        BIGINT,
    net_tx        BIGINT,
    disk_read     BIGINT,
    disk_write    BIGINT,
    load1         DOUBLE PRECISION,
    load5         DOUBLE PRECISION,
    load15        DOUBLE PRECISION,
    cpu_user      DOUBLE PRECISION,
    cpu_system    DOUBLE PRECISION,
    cpu_iowait    DOUBLE PRECISION,
    cpu_steal     DOUBLE PRECISION,
    disk_util     DOUBLE PRECISION,
    mem_available DOUBLE PRECISION,
    mem_buffers   DOUBLE PRECISION,
    mem_cached    DOUBLE PRECISION,
    mem_free      DOUBLE PRECISION
);
CREATE TABLE system_metrics_1h_new (LIKE system_metrics_1m_new INCLUDING DEFAULTS);

SELECT create_hypertable('system_metrics_1m_new', 'bucket', chunk_time_interval => INTERVAL '1 day');
SELECT create_hypertable('system_metrics_1h_new', 'bucket', chunk_time_interval => INTERVAL '7 days');

-- Copy the existing rollups across when they exist. On a fresh install they do not
-- (the ladder was only ever created at startup, never in a migration), so this must not
-- assume them — a missing view here would make the hub fail to start.
DO $$
BEGIN
    IF to_regclass('public.system_metrics_1m') IS NOT NULL THEN
        INSERT INTO system_metrics_1m_new
            (bucket, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
             disk_used, disk_total, net_rx, net_tx, disk_read, disk_write, load1, load5,
             load15, cpu_user, cpu_system, cpu_iowait, cpu_steal, disk_util,
             mem_available, mem_buffers, mem_cached, mem_free)
        SELECT bucket, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
               disk_used, disk_total, net_rx, net_tx, disk_read, disk_write, load1, load5,
               load15, cpu_user, cpu_system, cpu_iowait, cpu_steal, disk_util,
               mem_available, mem_buffers, mem_cached, mem_free
        FROM system_metrics_1m;
    END IF;
    IF to_regclass('public.system_metrics_1h') IS NOT NULL THEN
        INSERT INTO system_metrics_1h_new
            (bucket, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
             disk_used, disk_total, net_rx, net_tx, disk_read, disk_write, load1, load5,
             load15, cpu_user, cpu_system, cpu_iowait, cpu_steal, disk_util,
             mem_available, mem_buffers, mem_cached, mem_free)
        SELECT bucket, system_id, cpu_percent, mem_used, mem_total, swap_used, swap_total,
               disk_used, disk_total, net_rx, net_tx, disk_read, disk_write, load1, load5,
               load15, cpu_user, cpu_system, cpu_iowait, cpu_steal, disk_util,
               mem_available, mem_buffers, mem_cached, mem_free
        FROM system_metrics_1h;
    END IF;
END $$;

DROP MATERIALIZED VIEW IF EXISTS system_metrics_1h CASCADE;
DROP MATERIALIZED VIEW IF EXISTS system_metrics_15m CASCADE;
DROP MATERIALIZED VIEW IF EXISTS system_metrics_5m CASCADE;
DROP MATERIALIZED VIEW IF EXISTS system_metrics_1m CASCADE;
DROP MATERIALIZED VIEW IF EXISTS container_metrics_1h CASCADE;
DROP MATERIALIZED VIEW IF EXISTS container_metrics_15m CASCADE;
DROP MATERIALIZED VIEW IF EXISTS container_metrics_5m CASCADE;
DROP MATERIALIZED VIEW IF EXISTS container_metrics_1m CASCADE;

ALTER TABLE system_metrics_1m_new RENAME TO system_metrics_1m;
ALTER TABLE system_metrics_1h_new RENAME TO system_metrics_1h;

-- `system_id` leads so one index serves BOTH the fill job's conflict check and the read
-- path's `WHERE system_id = ? AND bucket > ?`; a hypertable only requires that a unique
-- index CONTAIN the partitioning column, not that it lead with it. Carrying a second
-- index for the lookup is what made the k8s rollup rows ~380 bytes each, over half of it
-- index.
CREATE UNIQUE INDEX idx_sm_1m_key ON system_metrics_1m (system_id, bucket DESC);
CREATE UNIQUE INDEX idx_sm_1h_key ON system_metrics_1h (system_id, bucket DESC);

-- ------------------------------------------------------- kube: consistent tier names
-- `kube_container_stats` is sampled at the cluster cadence (60s), so it already IS the
-- 1-minute tier — it keeps pod/container/labels and serves both drill-down and the
-- short-range charts. Renaming it makes the suffix mean the same thing in both families.
ALTER TABLE kube_container_stats RENAME TO kube_metrics_1m;
ALTER INDEX idx_kube_container_sys_time RENAME TO idx_km_1m_sys_time;
ALTER INDEX idx_kube_container_sys_ns_time RENAME TO idx_km_1m_sys_ns_time;

-- Renaming a hypertable and its indexes is fine with compression on; CREATING one is
-- NOT. `CREATE UNIQUE INDEX` on a hypertable that has compression enabled fails with
-- "operation not supported on hypertables that have compression enabled", because
-- uniqueness cannot be enforced across compressed chunks. This table has had compression
-- enabled since 3.2.0, so its existing indexes are carried across as they are.
--
-- That costs an index-layout optimisation (leading with system_id would let one index
-- serve both the fill job's conflict check and the read path, saving roughly a fifth of
-- the rollup's size). Doing it here would mean decompressing every chunk inside the
-- migration transaction — far more risk than the saving is worth. It belongs in its own
-- migration that decompresses, rebuilds and re-enables deliberately.
ALTER TABLE kube_rollup_1h RENAME TO kube_metrics_1h;
ALTER INDEX idx_kube_1h_key RENAME TO idx_km_1h_key;
ALTER INDEX idx_kube_1h_sys_bucket RENAME TO idx_km_1h_sys_bucket;

-- The 5-minute rung goes away with the ladder it belonged to. Nothing is lost that the
-- 1m tier (2 days) and the 1h tier (a year) do not already cover.
DROP TABLE IF EXISTS kube_rollup_5m;

-- ------------------------------------------------------ every tier names its cadence
-- A table called `system_metrics` says nothing about what one row means; next to
-- `system_metrics_1m` it is actively confusing, because the reader cannot tell whether
-- the unsuffixed one is finer or coarser. The suffix is the tier's DESIGN resolution,
-- not a promise about the live setting: both sampling cadences are admin-configurable
-- (hosts 1-3600s, clusters 5-3600s), so an operator who sets hosts to 10s keeps a table
-- named _5s. That is the same convention Graphite and Netdata use for their tiers.
--
-- `heartbeats` deliberately keeps its name: a row there is one service check, not a
-- sample on a clock, and its interval is per-monitor (60s to 14h on this install). A
-- time suffix would state a number that does not exist.
ALTER TABLE system_metrics RENAME TO system_metrics_5s;
ALTER INDEX idx_system_metrics_sys_time RENAME TO idx_sm_5s_sys_time;
ALTER TABLE container_metrics RENAME TO container_metrics_5s;
ALTER INDEX idx_container_metrics_sys_time RENAME TO idx_cm_5s_sys_time;

-- Progress is now tracked for every family, not just kube.
ALTER TABLE kube_rollup_progress RENAME TO rollup_progress;
DELETE FROM rollup_progress;
