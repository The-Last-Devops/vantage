use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::web::internal;
use crate::AppState;

#[derive(Serialize)]
pub struct MonitorRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub namespace: String,
    pub interval_secs: i32,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub up: Option<bool>,
    pub latency_ms: Option<i32>,
    pub last_check: Option<chrono::DateTime<chrono::Utc>>,
    pub message: Option<String>,
    /// Uptime % over the last 24 hours (avg of up-beats). None if no beats yet.
    pub uptime_24h: Option<f64>,
    /// Last ~40 heartbeats (oldest→newest) for the row's mini uptime bar.
    pub recent: Vec<bool>,
    /// 24h trend: one slot per hour (oldest→newest, 24 slots). Some(true)=all up
    /// that hour, Some(false)=had a down beat, None=no beats in that hour.
    pub trend_24h: Vec<Option<bool>>,
}

/// GET /api/monitors — each monitor (scoped to the caller's namespaces) plus
/// its latest heartbeat + a recent-beats sparkline. Admins see every monitor.
pub async fn list_monitors(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<MonitorRow>>, StatusCode> {
    let monitors: Vec<(Uuid, String, String, String, String, i32, bool, sqlx::types::Json<serde_json::Value>)> = sqlx::query_as(
        "SELECT m.id, m.name, m.kind::text, m.target, n.name, m.interval_secs, m.enabled, m.config \
         FROM monitors m JOIN namespaces n ON n.id = m.namespace_id \
         WHERE $1 OR m.namespace_id IN ( \
            SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY m.name",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    // Latest heartbeat for ALL monitors in ONE query (was N+1). DISTINCT ON + the
    // (monitor_id, time DESC) index makes this a fast per-monitor index scan.
    let ids: Vec<Uuid> = monitors.iter().map(|m| m.0).collect();
    #[allow(clippy::type_complexity)]
    let beat_rows: Vec<(
        Uuid,
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<i32>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT DISTINCT ON (monitor_id) monitor_id, time, up, latency_ms, message \
         FROM heartbeats WHERE monitor_id = ANY($1) ORDER BY monitor_id, time DESC",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    #[allow(clippy::type_complexity)]
    let mut latest: std::collections::HashMap<
        Uuid,
        (
            chrono::DateTime<chrono::Utc>,
            bool,
            Option<i32>,
            Option<String>,
        ),
    > = std::collections::HashMap::with_capacity(beat_rows.len());
    for (mid, t, up, lat, msg) in beat_rows {
        latest.insert(mid, (t, up, lat, msg));
    }

    // Last ~40 beats per monitor (oldest→newest) for the mini uptime bar — ONE
    // windowed query for all monitors.
    let recent_rows: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT monitor_id, up FROM ( \
           SELECT monitor_id, up, time, \
                  row_number() OVER (PARTITION BY monitor_id ORDER BY time DESC) AS rn \
           FROM heartbeats WHERE monitor_id = ANY($1) \
         ) t WHERE rn <= 40 ORDER BY monitor_id, time ASC",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    let mut recent: std::collections::HashMap<Uuid, Vec<bool>> = std::collections::HashMap::new();
    for (mid, up) in recent_rows {
        recent.entry(mid).or_default().push(up);
    }

    // 24h uptime % per monitor — avg of up-beats over the last 24 hours, in ONE
    // grouped query (mirrors the detail page's fixed-window figures).
    let uptime_rows: Vec<(Uuid, Option<f64>)> = sqlx::query_as(
        "SELECT monitor_id, avg((up)::int)::float8 * 100 \
         FROM heartbeats WHERE monitor_id = ANY($1) AND time > now() - interval '24 hours' \
         GROUP BY monitor_id",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    let mut uptime_24h: std::collections::HashMap<Uuid, f64> =
        std::collections::HashMap::with_capacity(uptime_rows.len());
    for (mid, pct) in uptime_rows {
        if let Some(p) = pct {
            uptime_24h.insert(mid, p);
        }
    }

    // 24h trend — one bucket per hour for the last 24 hours. bool_and(up) marks
    // an hour "down" if any beat in it failed; hours with no beats stay None.
    let trend_rows: Vec<(Uuid, i32, Option<bool>)> = sqlx::query_as(
        "SELECT monitor_id, \
                floor(extract(epoch FROM (now() - time)) / 3600)::int AS hours_ago, \
                bool_and(up) AS all_up \
         FROM heartbeats WHERE monitor_id = ANY($1) AND time > now() - interval '24 hours' \
         GROUP BY monitor_id, hours_ago",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    let mut trend: std::collections::HashMap<Uuid, [Option<bool>; 24]> =
        std::collections::HashMap::new();
    for (mid, hours_ago, all_up) in trend_rows {
        if !(0..24).contains(&hours_ago) {
            continue;
        }
        // oldest (23h ago) → index 0, current hour → index 23
        trend.entry(mid).or_insert([None; 24])[(23 - hours_ago) as usize] = all_up;
    }

    let mut rows = Vec::with_capacity(monitors.len());
    for (id, name, kind, target, namespace, interval_secs, enabled, config) in monitors {
        let (last_check, up, latency_ms, message) = match latest.remove(&id) {
            Some((t, up, lat, msg)) => (Some(t), Some(up), lat, msg),
            None => (None, None, None, None),
        };
        // The list/cards never need credentials — editors get full values from
        // the detail endpoint — so redact every secret in config (push token,
        // header creds, auth/redis passwords) and the target for everyone.
        let mut config = config.0;
        super::redact_monitor_config(&mut config);
        let target = super::mask_target(&target);
        rows.push(MonitorRow {
            id,
            name,
            kind,
            target,
            namespace,
            interval_secs,
            enabled,
            config,
            up,
            latency_ms,
            last_check,
            message,
            uptime_24h: uptime_24h.remove(&id),
            recent: recent.remove(&id).unwrap_or_default(),
            trend_24h: trend
                .remove(&id)
                .map(|a| a.to_vec())
                .unwrap_or_else(|| vec![None; 24]),
        });
    }
    Ok(Json(rows))
}
