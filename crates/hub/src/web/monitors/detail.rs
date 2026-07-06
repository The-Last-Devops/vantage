use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use super::load_monitor;
use crate::auth::CurrentUser;
use crate::web::internal;
use crate::AppState;

#[derive(Serialize)]
pub struct MonitorDetail {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub workspace: String,
    pub interval_secs: i32,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub up: Option<bool>,
    pub latency_ms: Option<i32>,
    pub message: Option<String>,
    pub last_check: Option<chrono::DateTime<chrono::Utc>>,
    /// When the current up/down status began (start of the latest unbroken run).
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    pub uptime_30d: Option<f64>,
}

async fn uptime_pct(state: &AppState, id: Uuid, interval: &str) -> Option<f64> {
    sqlx::query_as::<_, (Option<f64>,)>(&format!(
        "SELECT avg((up)::int)::float8 * 100 FROM heartbeats \
         WHERE monitor_id = $1 AND time > now() - interval '{interval}'"
    ))
    .bind(id)
    .fetch_optional(&state.data)
    .await
    .ok()
    .flatten()
    .and_then(|(p,)| p)
}

/// GET /api/monitors/:id — one monitor with status, current-status duration and
/// uptime percentages, for the detail page.
pub async fn monitor_detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<MonitorDetail>, StatusCode> {
    let (workspace, name, kind, target, interval_secs, enabled, mut config) =
        load_monitor(&state, &user, id).await?;

    // config (push token, header creds, auth/redis passwords) and the target
    // (DB connection-string passwords) are credentials — show them only to those
    // who can edit this monitor (so they can configure it); redact for plain
    // viewers. One role check gates both.
    let can_edit: (bool,) = if user.is_admin {
        (true,)
    } else {
        sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM memberships me \
             JOIN monitors mo ON mo.workspace_id = me.workspace_id \
             WHERE mo.id = $1 AND me.user_id = $2 AND me.role IN ('editor', 'owner'))",
        )
        .bind(id)
        .bind(user.id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?
    };
    let target = if can_edit.0 {
        target
    } else {
        super::redact_monitor_config(&mut config);
        super::mask_target(&target)
    };

    let latest: Option<(
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<i32>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT time, up, latency_ms, message FROM heartbeats \
         WHERE monitor_id = $1 ORDER BY time DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.data)
    .await
    .map_err(internal)?;

    let (last_check, up, latency_ms, message) = match &latest {
        Some((t, u, lat, msg)) => (Some(*t), Some(*u), *lat, msg.clone()),
        None => (None, None, None, None),
    };

    // Start of the current run = the first beat after the last opposite-status beat.
    let since: Option<chrono::DateTime<chrono::Utc>> = if let Some((_, cur_up, _, _)) = latest {
        let last_flip: Option<(Option<chrono::DateTime<chrono::Utc>>,)> =
            sqlx::query_as("SELECT max(time) FROM heartbeats WHERE monitor_id = $1 AND up <> $2")
                .bind(id)
                .bind(cur_up)
                .fetch_optional(&state.data)
                .await
                .map_err(internal)?;
        let flip = last_flip.and_then(|(t,)| t);
        sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>,)>(
            "SELECT min(time) FROM heartbeats \
             WHERE monitor_id = $1 AND ($2::timestamptz IS NULL OR time > $2)",
        )
        .bind(id)
        .bind(flip)
        .fetch_optional(&state.data)
        .await
        .map_err(internal)?
        .and_then(|(t,)| t)
    } else {
        None
    };

    Ok(Json(MonitorDetail {
        id,
        name,
        kind,
        target,
        workspace,
        interval_secs,
        enabled,
        config,
        up,
        latency_ms,
        message,
        last_check,
        since,
        uptime_24h: uptime_pct(&state, id, "24 hours").await,
        uptime_7d: uptime_pct(&state, id, "7 days").await,
        uptime_30d: uptime_pct(&state, id, "30 days").await,
    }))
}
