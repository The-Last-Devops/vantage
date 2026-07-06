-- Kubernetes cluster-state time series, pushed by the cluster-scoped agent
-- (AGENT_KIND=k8s-cluster) to POST /pub/kube. Linked to the config DB only by
-- system_id (a systems row of kind 'k8s-cluster' = one cluster). Retention +
-- compression are configured at startup in data_admin::setup (idempotent).

-- Per-namespace pod tallies, one row per namespace per snapshot.
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

-- Per-deployment replica health, one row per deployment per snapshot.
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
