use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use super::{hb_range, load_monitor};
use crate::auth::CurrentUser;
use crate::web::{internal, RangeQuery};
use crate::AppState;

#[derive(Serialize)]
pub struct HeartbeatSeries {
    pub t: Vec<i64>,
    pub latency: Vec<Option<f64>>,
    /// 1 = up for the whole bucket, 0 = at least one down beat, null = no data.
    pub up: Vec<Option<f64>>,
}

/// GET /api/monitors/:id/heartbeats?range= — bucketed latency + up/down series.
pub async fn monitor_heartbeats(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<HeartbeatSeries>, StatusCode> {
    load_monitor(&state, &user, id).await?; // authorize
    let (interval, bucket) = hb_range(&q.range);
    // gapfill → empty buckets come back as NULL so the chart/strip show blanks for
    // the whole window instead of stretching a few points across it.
    let rows: Vec<(chrono::DateTime<chrono::Utc>, Option<f64>, Option<f64>)> =
        sqlx::query_as(&format!(
            "SELECT time_bucket_gapfill('{bucket}', time) AS b, \
                avg(latency_ms)::float8 AS latency, \
                min((up)::int)::float8 AS up \
         FROM heartbeats \
         WHERE monitor_id = $1 AND time >= now() - interval '{interval}' AND time <= now() \
         GROUP BY b ORDER BY b"
        ))
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    let mut s = HeartbeatSeries {
        t: Vec::with_capacity(rows.len()),
        latency: Vec::with_capacity(rows.len()),
        up: Vec::with_capacity(rows.len()),
    };
    for (b, latency, up) in rows {
        s.t.push(b.timestamp());
        s.latency.push(latency);
        s.up.push(up);
    }
    Ok(Json(s))
}

#[derive(Serialize)]
pub struct MonitorEvent {
    pub at: chrono::DateTime<chrono::Utc>,
    pub up: bool,
    pub message: Option<String>,
}

/// GET /api/monitors/:id/events?range= — status transitions (up↔down) for the
/// monitor, newest first. The frontend pairs down→up to show incident durations.
pub async fn monitor_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<Vec<MonitorEvent>>, StatusCode> {
    load_monitor(&state, &user, id).await?;
    let (interval, _) = hb_range(&q.range);
    let rows: Vec<(chrono::DateTime<chrono::Utc>, bool, Option<String>)> = sqlx::query_as(&format!(
        "WITH h AS ( \
           SELECT time, up, message, lag(up) OVER (ORDER BY time) AS prev \
           FROM heartbeats WHERE monitor_id = $1 AND time > now() - interval '{interval}' \
         ) SELECT time, up, message FROM h WHERE prev IS NULL OR up <> prev ORDER BY time DESC LIMIT 200"
    ))
    .bind(id)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(at, up, message)| MonitorEvent { at, up, message })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct GlobalEvent {
    pub monitor_id: Uuid,
    pub name: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub up: bool,
    pub message: Option<String>,
}

/// GET /api/events?range= — recent status transitions across all the caller's
/// monitors (newest first), for the Services overview events feed.
pub async fn recent_events(
    State(state): State<AppState>,
    user: CurrentUser,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<Vec<GlobalEvent>>, StatusCode> {
    // monitors the caller can see (id → name) from the config DB
    let mons: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT m.id, m.name FROM monitors m \
         WHERE $1 OR m.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2)",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    let names: std::collections::HashMap<Uuid, String> = mons.into_iter().collect();
    let ids: Vec<Uuid> = names.keys().copied().collect();
    let (interval, _) = hb_range(&q.range);
    let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, bool, Option<String>)> =
        sqlx::query_as(&format!(
            "WITH h AS ( \
           SELECT monitor_id, time, up, message, \
                  lag(up) OVER (PARTITION BY monitor_id ORDER BY time) AS prev \
           FROM heartbeats WHERE monitor_id = ANY($1) AND time > now() - interval '{interval}' \
         ) SELECT monitor_id, time, up, message FROM h WHERE prev IS NULL OR up <> prev \
           ORDER BY time DESC LIMIT 100"
        ))
        .bind(&ids)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .filter_map(|(mid, at, up, message)| {
                names.get(&mid).map(|name| GlobalEvent {
                    monitor_id: mid,
                    name: name.clone(),
                    at,
                    up,
                    message,
                })
            })
            .collect(),
    ))
}
