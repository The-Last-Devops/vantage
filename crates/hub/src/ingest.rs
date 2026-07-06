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
use shared::{IngestAck, MetricsReport, API_KEY_HEADER};
use uuid::Uuid;

use crate::AppState;

pub async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(report): Json<MetricsReport>,
) -> Result<Json<IngestAck>, StatusCode> {
    let key = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Resolve the (reusable) API key -> its workspace.
    let row: (Uuid, Uuid) = sqlx::query_as("SELECT id, workspace_id FROM api_keys WHERE key = $1")
        .bind(key)
        .fetch_optional(&state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "config DB error during ingest");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let (key_id, workspace_id) = row;

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

    // Auto-register / update the system identified by (key, hostname).
    let system: (Uuid,) = sqlx::query_as(
        "INSERT INTO systems (workspace_id, key_id, name, hostname, kernel, cpu_model, cpu_cores, agent_version, kind, cluster, last_seen) \
         VALUES ($1, $2, $3, $3, $4, $5, $6, $7, $8, $9, now()) \
         ON CONFLICT (key_id, hostname) DO UPDATE SET \
            last_seen = now(), kernel = $4, cpu_model = $5, cpu_cores = $6, agent_version = $7, kind = $8, cluster = $9 \
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
