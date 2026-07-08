-- Per-container resource usage + pod metadata time series, pushed by the
-- cluster-scoped agent (AGENT_KIND=k8s-cluster) to POST /pub/kube. One row per
-- container per snapshot: the granular unit, so the hub can aggregate on READ by
-- any dimension (namespace, workload, or label) instead of storing pre-rolled
-- totals. Linked to the config DB only by system_id (a 'k8s-cluster' system).
-- Retention + compression are configured at startup in data_admin::setup.
CREATE TABLE kube_container_stats (
    time           TIMESTAMPTZ NOT NULL,
    system_id      UUID NOT NULL,
    namespace      TEXT NOT NULL,
    pod            TEXT NOT NULL,
    container      TEXT NOT NULL,
    node           TEXT NOT NULL DEFAULT '',
    phase          TEXT NOT NULL DEFAULT '',
    -- Resolved top-level controller (pod → ReplicaSet → Deployment); "" for bare pods.
    workload       TEXT NOT NULL DEFAULT '',
    workload_kind  TEXT NOT NULL DEFAULT '',
    cpu_millicores BIGINT NOT NULL DEFAULT 0,
    mem_bytes      BIGINT NOT NULL DEFAULT 0,
    restarts       INTEGER NOT NULL DEFAULT 0,
    -- Full pod labels, for grouping by label (e.g. labels->>'app'). Duplicated
    -- across the pod's containers; TimescaleDB compression dedupes it well.
    labels         JSONB NOT NULL DEFAULT '{}'::jsonb
);
SELECT create_hypertable('kube_container_stats', 'time');
CREATE INDEX idx_kube_container_sys_time    ON kube_container_stats (system_id, time DESC);
CREATE INDEX idx_kube_container_sys_ns_time ON kube_container_stats (system_id, namespace, time DESC);
