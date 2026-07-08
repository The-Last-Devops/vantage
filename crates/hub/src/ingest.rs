//! Agent metrics ingest endpoint.
//!
//! Flow: authenticate the agent by its API key (config DB) -> resolve the owning
//! system -> write the sample into the data DB hypertable.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use shared::{IngestAck, KubeReport, MetricsReport, API_KEY_HEADER};
use uuid::Uuid;

use crate::AppState;

/// Resolve the `x-api-key` header to `(key_id, workspace_id)`. Reusable keys enroll
/// many systems, so this is the single auth check every push path starts with.
async fn resolve_key(state: &AppState, headers: &HeaderMap) -> Result<(Uuid, Uuid), StatusCode> {
    let key = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let row: (Uuid, Uuid) = sqlx::query_as("SELECT id, workspace_id FROM api_keys WHERE key = $1")
        .bind(key)
        .fetch_optional(&state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "config DB error resolving api key");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(row)
}

pub async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<MetricsReport>,
) -> Result<Json<IngestAck>, StatusCode> {
    let (key_id, workspace_id) = resolve_key(&state, &headers).await?;

    let hostname = if report.hostname.is_empty() {
        "unknown".to_string()
    } else {
        report.hostname.clone()
    };

    // Classification: empty kind defaults to "node"; cluster only meaningful for k8s.
    let kind = match report.kind.as_str() {
        "docker" | "k8s" => report.kind.as_str(),
        _ => "node",
    };
    let cluster = if kind == "k8s" && !report.cluster.is_empty() {
        Some(report.cluster.as_str())
    } else {
        None
    };

    // Auto-register / update the system identified by (workspace, hostname) — NOT by
    // key: re-enrolling a host under a new key must update the SAME row, not create a
    // duplicate. The owning key_id follows the latest report.
    let system: (Uuid,) = sqlx::query_as(
        "INSERT INTO systems (workspace_id, key_id, name, hostname, kernel, cpu_model, cpu_cores, agent_version, kind, cluster, last_seen) \
         VALUES ($1, $2, $3, $3, $4, $5, $6, $7, $8, $9, now()) \
         ON CONFLICT (workspace_id, hostname) DO UPDATE SET \
            key_id = EXCLUDED.key_id, last_seen = now(), kernel = $4, cpu_model = $5, cpu_cores = $6, agent_version = $7, kind = $8, cluster = $9 \
         RETURNING id",
    )
    .bind(workspace_id)
    .bind(key_id)
    .bind(&hostname)
    .bind(&report.kernel)
    .bind(&report.cpu_model)
    .bind(report.cpu_cores as i32)
    .bind(&report.agent_version)
    .bind(kind)
    .bind(cluster)
    .fetch_one(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "system upsert during ingest");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let system_id = system.0;
    let ts = chrono::DateTime::from_timestamp(report.ts, 0).unwrap_or_else(chrono::Utc::now);

    // Write the sample into the data DB. system_id is the cross-DB link (no JOINs).
    sqlx::query(
        r#"
        INSERT INTO system_metrics (
            time, system_id, cpu_percent, mem_used, mem_total,
            swap_used, swap_total, disk_used, disk_total,
            net_rx, net_tx, load1, uptime, temps,
            disk_read, disk_write, gpus, load5, load15,
            cpu_user, cpu_system, cpu_iowait, cpu_steal, disk_util,
            mem_available, mem_buffers, mem_cached, mem_free
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28)
        "#,
    )
    .bind(ts)
    .bind(system_id)
    .bind(report.cpu_percent as f64)
    .bind(report.mem_used as i64)
    .bind(report.mem_total as i64)
    .bind(report.swap_used as i64)
    .bind(report.swap_total as i64)
    .bind(report.disk_used as i64)
    .bind(report.disk_total as i64)
    .bind(report.net_rx as i64)
    .bind(report.net_tx as i64)
    .bind(report.load1)
    .bind(report.uptime as i64)
    .bind(sqlx::types::Json(&report.temps))
    .bind(report.disk_read as i64)
    .bind(report.disk_write as i64)
    .bind(sqlx::types::Json(&report.gpus))
    .bind(report.load5)
    .bind(report.load15)
    .bind(report.cpu_user as f64)
    .bind(report.cpu_system as f64)
    .bind(report.cpu_iowait as f64)
    .bind(report.cpu_steal as f64)
    .bind(report.disk_util as f64)
    .bind(report.mem_available as i64)
    .bind(report.mem_buffers as i64)
    .bind(report.mem_cached as i64)
    .bind(report.mem_free as i64)
    .execute(&state.data)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "data DB error during ingest");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Per-container stats (best-effort).
    for c in &report.containers {
        let _ = sqlx::query(
            "INSERT INTO container_metrics (time, system_id, name, cpu_percent, mem_used, net_rx, net_tx) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(ts)
        .bind(system_id)
        .bind(&c.name)
        .bind(c.cpu_percent as f64)
        .bind(c.mem_used as i64)
        .bind(c.net_rx as i64)
        .bind(c.net_tx as i64)
        .execute(&state.data)
        .await;
    }

    Ok(Json(IngestAck {
        ok: true,
        next_interval_secs: 0, // 0 => agent keeps its current interval
        // Advertise the hub's build so `auto`-channel agents can follow it.
        hub_build: Some(env!("GIT_SHA").to_string()),
    }))
}

/// `POST /pub/kube` — cluster-state ingest from the cluster-scoped agent
/// (`AGENT_KIND=k8s-cluster`). Authenticates the same way as host ingest, upserts a
/// `systems` row of kind `k8s-cluster` (one per cluster, keyed by cluster name), then
/// writes the per-namespace and per-deployment rows into the data DB.
pub async fn ingest_kube(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<KubeReport>,
) -> Result<Json<IngestAck>, StatusCode> {
    let (key_id, workspace_id) = resolve_key(&state, &headers).await?;

    // The cluster name is the cluster's stable identity (its hostname in `systems`).
    let cluster = if report.cluster.is_empty() {
        "default"
    } else {
        report.cluster.as_str()
    };

    // Auto-register / update the cluster as a system of kind 'k8s-cluster', keyed by
    // (workspace, cluster-name) so re-enrolling under a new key updates the same row.
    let system: (Uuid,) = sqlx::query_as(
        "INSERT INTO systems (workspace_id, key_id, name, hostname, kind, cluster, agent_version, k8s_version, last_seen) \
         VALUES ($1, $2, $3, $3, 'k8s-cluster', $3, $4, $5, now()) \
         ON CONFLICT (workspace_id, hostname) DO UPDATE SET \
            key_id = EXCLUDED.key_id, last_seen = now(), kind = 'k8s-cluster', cluster = $3, agent_version = $4, k8s_version = $5 \
         RETURNING id",
    )
    .bind(workspace_id)
    .bind(key_id)
    .bind(cluster)
    .bind(&report.agent_version)
    .bind(&report.k8s_version)
    .fetch_one(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "cluster upsert during kube ingest");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let system_id = system.0;
    let ts = chrono::DateTime::from_timestamp(report.ts, 0).unwrap_or_else(chrono::Utc::now);

    // Per-namespace tallies.
    for n in &report.namespaces {
        if let Err(e) = sqlx::query(
            "INSERT INTO kube_namespace_stats \
             (time, system_id, namespace, phase, pods_total, pods_running, pods_pending, pods_failed, pods_succeeded, restarts) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(ts)
        .bind(system_id)
        .bind(&n.name)
        .bind(&n.phase)
        .bind(n.pods_total as i32)
        .bind(n.pods_running as i32)
        .bind(n.pods_pending as i32)
        .bind(n.pods_failed as i32)
        .bind(n.pods_succeeded as i32)
        .bind(n.restarts as i32)
        .execute(&state.data)
        .await
        {
            tracing::error!(error = %e, "kube namespace insert");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Per-deployment replica health.
    for d in &report.deployments {
        if let Err(e) = sqlx::query(
            "INSERT INTO kube_deployment_stats \
             (time, system_id, namespace, name, desired, ready, available, updated) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(ts)
        .bind(system_id)
        .bind(&d.namespace)
        .bind(&d.name)
        .bind(d.desired as i32)
        .bind(d.ready as i32)
        .bind(d.available as i32)
        .bind(d.updated as i32)
        .execute(&state.data)
        .await
        {
            tracing::error!(error = %e, "kube deployment insert");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Per-container usage + pod metadata. Potentially hundreds of rows per push,
    // so insert them in ONE statement via UNNEST (parallel arrays) instead of a
    // row-per-round-trip loop. `labels` is passed as JSON text[] and cast to jsonb.
    if !report.containers.is_empty() {
        let n = report.containers.len();
        let mut ns = Vec::with_capacity(n);
        let mut pod = Vec::with_capacity(n);
        let mut container = Vec::with_capacity(n);
        let mut node = Vec::with_capacity(n);
        let mut phase = Vec::with_capacity(n);
        let mut workload = Vec::with_capacity(n);
        let mut workload_kind = Vec::with_capacity(n);
        let mut cpu = Vec::with_capacity(n);
        let mut mem = Vec::with_capacity(n);
        let mut restarts = Vec::with_capacity(n);
        let mut labels = Vec::with_capacity(n);
        for c in &report.containers {
            ns.push(c.namespace.clone());
            pod.push(c.pod.clone());
            container.push(c.container.clone());
            node.push(c.node.clone());
            phase.push(c.phase.clone());
            workload.push(c.workload.clone());
            workload_kind.push(c.workload_kind.clone());
            cpu.push(c.cpu_millicores as i64);
            mem.push(c.mem_bytes as i64);
            restarts.push(c.restarts as i32);
            labels.push(serde_json::to_string(&c.labels).unwrap_or_else(|_| "{}".into()));
        }
        if let Err(e) = sqlx::query(
            "INSERT INTO kube_container_stats \
             (time, system_id, namespace, pod, container, node, phase, workload, workload_kind, cpu_millicores, mem_bytes, restarts, labels) \
             SELECT $1, $2, t.ns, t.pod, t.container, t.node, t.phase, t.workload, t.workload_kind, t.cpu, t.mem, t.restarts, t.labels::jsonb \
             FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::text[], $8::text[], $9::text[], $10::bigint[], $11::bigint[], $12::int[], $13::text[]) \
                  AS t(ns, pod, container, node, phase, workload, workload_kind, cpu, mem, restarts, labels)",
        )
        .bind(ts)
        .bind(system_id)
        .bind(&ns)
        .bind(&pod)
        .bind(&container)
        .bind(&node)
        .bind(&phase)
        .bind(&workload)
        .bind(&workload_kind)
        .bind(&cpu)
        .bind(&mem)
        .bind(&restarts)
        .bind(&labels)
        .execute(&state.data)
        .await
        {
            tracing::error!(error = %e, "kube container insert");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Json(IngestAck {
        ok: true,
        next_interval_secs: 0,
        hub_build: Some(env!("GIT_SHA").to_string()),
    }))
}

/// Optional Uptime-Kuma-style query params for a push: `?status=up|down&msg=...&ping=<ms>`.
#[derive(Deserialize)]
pub struct PushQuery {
    pub status: Option<String>,
    pub msg: Option<String>,
    pub ping: Option<f64>,
}

/// GET/POST /pub/push/:token — a push (passive) monitor. The external job calls
/// this on its own schedule; we record an "up" heartbeat (or "down" if
/// `?status=down`). The probe scheduler writes a "down" beat if no push arrives
/// within the monitor's interval. Returns a small JSON ack so a human hitting the
/// URL in a browser sees confirmation instead of a blank page.
pub async fn push(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(q): Query<PushQuery>,
) -> impl IntoResponse {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, name FROM monitors WHERE config->>'push_token' = $1 AND enabled = true",
    )
    .bind(&token)
    .fetch_optional(&state.config)
    .await
    .ok()
    .flatten();
    let Some((id, name)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "msg": "unknown or disabled push token" })),
        );
    };
    let up = q.status.as_deref() != Some("down");
    let msg = q.msg.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if up {
            "OK (push received)"
        } else {
            "down (reported by push)"
        }
        .into()
    });
    let latency = q.ping.map(|p| p.round() as i32);
    let _ = sqlx::query(
        "INSERT INTO heartbeats (time, monitor_id, up, latency_ms, status_code, message) \
         VALUES (now(), $1, $2, $3, NULL, $4)",
    )
    .bind(id)
    .bind(up)
    .bind(latency)
    .bind(&msg)
    .execute(&state.data)
    .await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "monitor": name, "up": up, "msg": msg })),
    )
}
