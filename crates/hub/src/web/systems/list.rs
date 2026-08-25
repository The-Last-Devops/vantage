use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::web::internal;
use crate::AppState;

#[derive(Serialize)]
pub struct SystemRow {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub kind: String,
    pub cluster: Option<String>,
    pub workspace: String,
    pub agent_version: Option<String>,
    /// Kubernetes server version — only for `k8s-cluster` systems; null otherwise.
    pub k8s_version: Option<String>,
    pub kernel: Option<String>,
    pub cpu_model: Option<String>,
    pub cpu_cores: Option<i32>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub cpu_percent: Option<f64>,
    pub mem_used: Option<i64>,
    pub mem_total: Option<i64>,
    pub disk_used: Option<i64>,
    pub disk_total: Option<i64>,
    pub disk_util: Option<f64>,
}

/// GET /api/systems — each server (in workspaces the caller can see) plus its
/// most recent sample. Latest metric is fetched from the data DB per server
/// (no cross-DB JOIN). Admins see every server.
pub async fn list_systems(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<SystemRow>>, StatusCode> {
    let servers: Vec<(
        Uuid,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT s.id, s.name, s.hostname, s.kind, s.cluster, n.name, s.agent_version, \
                s.kernel, s.cpu_model, s.cpu_cores, s.last_seen, s.k8s_version \
             FROM systems s JOIN workspaces n ON n.id = s.workspace_id \
             WHERE $1 OR s.workspace_id IN ( \
                SELECT workspace_id FROM memberships WHERE user_id = $2) \
             ORDER BY s.name",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    // Latest sample for ALL systems in ONE query (was N+1). One LATERAL LIMIT 1 per
    // system, not `DISTINCT ON` over `= ANY($1)`: the latter had to sort every matching
    // row in `system_metrics` — the raw tier, one row per host per 5s — just to keep the
    // newest per host. The FRESH bound then lets TimescaleDB exclude all but the newest
    // chunk; a host silent that long is offline and shows no numbers anyway (`last_seen`
    // comes from the config DB and is unaffected). This is `/api/systems`, which the
    // Overview / Systems / Clusters / Fleet pages all poll.
    let ids: Vec<Uuid> = servers.iter().map(|s| s.0).collect();
    let latest_rows: Vec<(Uuid, f64, i64, i64, Option<i64>, Option<i64>, Option<f64>)> = sqlx::query_as(
        "SELECT s.sid, m.cpu_percent, m.mem_used, m.mem_total, m.disk_used, m.disk_total, m.disk_util \
         FROM unnest($1::uuid[]) AS s(sid) \
         JOIN LATERAL (SELECT cpu_percent, mem_used, mem_total, disk_used, disk_total, disk_util \
                       FROM system_metrics WHERE system_id = s.sid \
                         AND time > now() - interval '6 hours' \
                       ORDER BY time DESC LIMIT 1) m ON true",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    #[allow(clippy::type_complexity)]
    let mut latest: std::collections::HashMap<
        Uuid,
        (f64, i64, i64, Option<i64>, Option<i64>, Option<f64>),
    > = std::collections::HashMap::with_capacity(latest_rows.len());
    for (sid, c, mu, mt, du, dt, dutil) in latest_rows {
        latest.insert(sid, (c, mu, mt, du, dt, dutil));
    }

    let mut rows = Vec::with_capacity(servers.len());
    for (
        id,
        name,
        hostname,
        kind,
        cluster,
        workspace,
        agent_version,
        kernel,
        cpu_model,
        cpu_cores,
        last_seen,
        k8s_version,
    ) in servers
    {
        let (cpu_percent, mem_used, mem_total, disk_used, disk_total, disk_util) =
            match latest.get(&id) {
                Some(&(c, u, t, du, dt, dutil)) => (Some(c), Some(u), Some(t), du, dt, dutil),
                None => (None, None, None, None, None, None),
            };
        rows.push(SystemRow {
            id,
            name,
            hostname,
            kind,
            cluster,
            workspace,
            agent_version,
            k8s_version,
            kernel,
            cpu_model,
            cpu_cores,
            last_seen,
            cpu_percent,
            mem_used,
            mem_total,
            disk_used,
            disk_total,
            disk_util,
        });
    }
    Ok(Json(rows))
}
