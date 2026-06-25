use super::*;

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
    /// Last ~40 heartbeats (oldest→newest) for the row's mini uptime bar.
    pub recent: Vec<bool>,
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

    let mut rows = Vec::with_capacity(monitors.len());
    for (id, name, kind, target, namespace, interval_secs, enabled, config) in monitors {
        let (last_check, up, latency_ms, message) = match latest.remove(&id) {
            Some((t, up, lat, msg)) => (Some(t), Some(up), lat, msg),
            None => (None, None, None, None),
        };
        // A push monitor's token is a write credential (anyone with it can post
        // beats). The list/cards never need it — only the detail page shows the
        // URL, gated to editors — so strip it here for everyone.
        let mut config = config.0;
        if let Some(o) = config.as_object_mut() {
            o.remove("push_token");
        }
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
            recent: recent.remove(&id).unwrap_or_default(),
        });
    }
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct MonitorDetail {
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
    pub message: Option<String>,
    pub last_check: Option<chrono::DateTime<chrono::Utc>>,
    /// When the current up/down status began (start of the latest unbroken run).
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    pub uptime_30d: Option<f64>,
}

/// True if the user may view the given monitor; returns its (namespace, name,
/// kind, target, interval, enabled, config) when so.
#[allow(clippy::type_complexity)]
async fn load_monitor(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<(String, String, String, String, i32, bool, serde_json::Value), StatusCode> {
    let row: Option<(
        String,
        String,
        String,
        String,
        i32,
        bool,
        sqlx::types::Json<serde_json::Value>,
    )> = sqlx::query_as(
        "SELECT n.name, m.name, m.kind::text, m.target, m.interval_secs, m.enabled, m.config \
             FROM monitors m JOIN namespaces n ON n.id = m.namespace_id \
             WHERE m.id = $1 AND ($2 OR m.namespace_id IN ( \
                SELECT namespace_id FROM memberships WHERE user_id = $3))",
    )
    .bind(id)
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    let (namespace, name, kind, target, interval_secs, enabled, config) =
        row.ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        namespace,
        name,
        kind,
        target,
        interval_secs,
        enabled,
        config.0,
    ))
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
    let (namespace, name, kind, target, interval_secs, enabled, mut config) =
        load_monitor(&state, &user, id).await?;

    // The push token is a write credential — show it only to those who can edit
    // this monitor (so they can configure the cron); strip it for plain viewers.
    if config.get("push_token").is_some() {
        let can_edit: (bool,) = if user.is_admin {
            (true,)
        } else {
            sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM memberships me \
                 JOIN monitors mo ON mo.namespace_id = me.namespace_id \
                 WHERE mo.id = $1 AND me.user_id = $2 AND me.role IN ('editor', 'owner'))",
            )
            .bind(id)
            .bind(user.id)
            .fetch_one(&state.config)
            .await
            .map_err(internal)?
        };
        if !can_edit.0 {
            if let Some(o) = config.as_object_mut() {
                o.remove("push_token");
            }
        }
    }

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
        namespace,
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

#[derive(Serialize)]
pub struct HeartbeatSeries {
    pub t: Vec<i64>,
    pub latency: Vec<Option<f64>>,
    /// 1 = up for the whole bucket, 0 = at least one down beat, null = no data.
    pub up: Vec<Option<f64>>,
}

/// (interval, bucket) for the heartbeat history chart — supports up to 30 days.
fn hb_range(range: &Option<String>) -> (&'static str, &'static str) {
    match range.as_deref() {
        Some("1h") => ("1 hour", "1 minute"),
        Some("6h") => ("6 hours", "5 minutes"),
        Some("7d") => ("7 days", "1 hour"),
        Some("30d") => ("30 days", "6 hours"),
        Some("90d") => ("90 days", "1 day"),
        Some("1y") => ("365 days", "1 day"),
        _ => ("24 hours", "15 minutes"),
    }
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
         WHERE $1 OR m.namespace_id IN (SELECT namespace_id FROM memberships WHERE user_id = $2)",
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
