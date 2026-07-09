-- Data DB: time-series metrics + service heartbeats. PostgreSQL + TimescaleDB.
-- Linked to the config DB only by IDs (system_id / monitor_id) at the app layer.
-- Consolidated schema (squashed 2026-07-09 for the v3 GA — was migrations 0001-0005).
-- Retention / compression / continuous aggregates are configured at startup in
-- data_admin::setup (idempotent), NOT here.

CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Host metrics pushed by agents.
CREATE TABLE system_metrics (
    time          TIMESTAMPTZ NOT NULL,
    system_id     UUID NOT NULL,
    cpu_percent   DOUBLE PRECISION NOT NULL,
    mem_used      BIGINT NOT NULL,
    mem_total     BIGINT NOT NULL,
    swap_used     BIGINT NOT NULL,
    swap_total    BIGINT NOT NULL,
    disk_used     BIGINT NOT NULL,
    disk_total    BIGINT NOT NULL,
    net_rx        BIGINT NOT NULL,
    net_tx        BIGINT NOT NULL,
    load1         DOUBLE PRECISION NOT NULL,
    load5         DOUBLE PRECISION,
    load15        DOUBLE PRECISION,
    cpu_user      DOUBLE PRECISION,
    cpu_system    DOUBLE PRECISION,
    cpu_iowait    DOUBLE PRECISION,
    cpu_steal     DOUBLE PRECISION,
    uptime        BIGINT NOT NULL,
    disk_read     BIGINT,
    disk_write    BIGINT,
    temps         JSONB,
    gpus          JSONB,
    disk_util     DOUBLE PRECISION,
    mem_available BIGINT,
    mem_buffers   BIGINT,
    mem_cached    BIGINT,
    mem_free      BIGINT
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

-- Kubernetes cluster-state series (cluster-scoped agent → POST /pub/kube).
CREATE TABLE kube_namespace_stats (
    time           TIMESTAMPTZ NOT NULL,
    system_id      UUID NOT NULL,
    namespace      TEXT NOT NULL,
    phase          TEXT,
    pods_total     INTEGER NOT NULL,
    pods_running   INTEGER NOT NULL,
    pods_pending   INTEGER NOT NULL,
    pods_failed    INTEGER NOT NULL,
    pods_succeeded INTEGER NOT NULL,
    restarts       INTEGER NOT NULL
);
SELECT create_hypertable('kube_namespace_stats', 'time');
CREATE INDEX idx_kube_ns_sys_time ON kube_namespace_stats (system_id, time DESC);

CREATE TABLE kube_deployment_stats (
    time        TIMESTAMPTZ NOT NULL,
    system_id   UUID NOT NULL,
    namespace   TEXT NOT NULL,
    name        TEXT NOT NULL,
    desired     INTEGER NOT NULL,
    ready       INTEGER NOT NULL,
    available   INTEGER NOT NULL,
    updated     INTEGER NOT NULL
);
SELECT create_hypertable('kube_deployment_stats', 'time');
CREATE INDEX idx_kube_deploy_sys_time ON kube_deployment_stats (system_id, time DESC);

-- Per-container usage + pod metadata (aggregated on read by ns / workload / label).
CREATE TABLE kube_container_stats (
    time           TIMESTAMPTZ NOT NULL,
    system_id      UUID NOT NULL,
    namespace      TEXT NOT NULL,
    pod            TEXT NOT NULL,
    container      TEXT NOT NULL,
    node           TEXT NOT NULL DEFAULT '',
    phase          TEXT NOT NULL DEFAULT '',
    workload       TEXT NOT NULL DEFAULT '',
    workload_kind  TEXT NOT NULL DEFAULT '',
    cpu_millicores BIGINT NOT NULL DEFAULT 0,
    mem_bytes      BIGINT NOT NULL DEFAULT 0,
    restarts       INTEGER NOT NULL DEFAULT 0,
    labels         JSONB NOT NULL DEFAULT '{}'::jsonb
);
SELECT create_hypertable('kube_container_stats', 'time');
CREATE INDEX idx_kube_container_sys_time    ON kube_container_stats (system_id, time DESC);
CREATE INDEX idx_kube_container_sys_ns_time ON kube_container_stats (system_id, namespace, time DESC);
