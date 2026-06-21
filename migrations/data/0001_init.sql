-- Data DB: time-series metrics + service heartbeats. PostgreSQL + TimescaleDB.
-- Linked to the config DB only by IDs (system_id / monitor_id) at the app layer.
-- Pre-release: single consolidated schema. Retention / compression / continuous
-- aggregates are configured at startup in data_admin::setup (idempotent).

CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Host metrics pushed by agents.
CREATE TABLE system_metrics (
    time        TIMESTAMPTZ NOT NULL,
    system_id   UUID NOT NULL,
    cpu_percent DOUBLE PRECISION NOT NULL,
    mem_used    BIGINT NOT NULL,
    mem_total   BIGINT NOT NULL,
    swap_used   BIGINT NOT NULL,
    swap_total  BIGINT NOT NULL,
    disk_used   BIGINT NOT NULL,
    disk_total  BIGINT NOT NULL,
    net_rx      BIGINT NOT NULL,
    net_tx      BIGINT NOT NULL,
    load1       DOUBLE PRECISION NOT NULL,
    uptime      BIGINT NOT NULL,
    disk_read   BIGINT,
    disk_write  BIGINT,
    temps       JSONB,
    gpus        JSONB
);
SELECT create_hypertable('system_metrics', 'time');
CREATE INDEX idx_system_metrics_sys_time ON system_metrics (system_id, time DESC);

-- Per-container resource usage (Beszel-style Docker stats).
CREATE TABLE container_metrics (
    time        TIMESTAMPTZ NOT NULL,
    system_id   UUID NOT NULL,
    name        TEXT NOT NULL,
    cpu_percent DOUBLE PRECISION NOT NULL,
    mem_used    BIGINT NOT NULL,
    net_rx      BIGINT NOT NULL,
    net_tx      BIGINT NOT NULL
);
SELECT create_hypertable('container_metrics', 'time');
CREATE INDEX idx_container_metrics_sys_time ON container_metrics (system_id, time DESC);

-- Service check results.
CREATE TABLE heartbeats (
    time        TIMESTAMPTZ NOT NULL,
    monitor_id  UUID NOT NULL,
    up          BOOLEAN NOT NULL,
    latency_ms  INTEGER,
    status_code INTEGER,
    message     TEXT
);
SELECT create_hypertable('heartbeats', 'time');
CREATE INDEX idx_heartbeats_monitor_time ON heartbeats (monitor_id, time DESC);
