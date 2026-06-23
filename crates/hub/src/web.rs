//! Server-rendered web UI + JSON endpoints feeding it.
//!
//! Minimal skeleton: a dashboard page (HTML + HTMX + uPlot via CDN) and a JSON
//! endpoint listing servers with their latest metric. Real templating (Askama/Maud),
//! auth, and namespaces come next.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::AppState;

/// True if the user may view the given server (admin / read-only admin, or a
/// member of its namespace).
pub async fn can_view_system(
    state: &AppState,
    user: &CurrentUser,
    system_id: Uuid,
) -> Result<bool, StatusCode> {
    if user.can_read_all() {
        return Ok(true);
    }
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM systems s \
         JOIN memberships m ON m.namespace_id = s.namespace_id \
         WHERE s.id = $1 AND m.user_id = $2",
    )
    .bind(system_id)
    .bind(user.id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    Ok(row.is_some())
}

#[derive(Serialize)]
pub struct MetricsHistory {
    pub t: Vec<i64>,
    pub cpu: Vec<f64>,
    pub mem_pct: Vec<f64>,
    /// Raw used memory in bytes (for byte-unit overlays alongside container stats).
    pub mem_used: Vec<f64>,
    pub disk_pct: Vec<f64>,
    pub net_rx: Vec<f64>,
    pub net_tx: Vec<f64>,
    /// Disk read / write throughput (bytes/sec).
    pub dr: Vec<f64>,
    pub dw: Vec<f64>,
    /// Load average 1 / 5 / 15 minutes.
    pub load1: Vec<f64>,
    pub load5: Vec<f64>,
    pub load15: Vec<f64>,
    /// CPU time breakdown (% of total): user / system / iowait / steal.
    pub cpu_user: Vec<f64>,
    pub cpu_system: Vec<f64>,
    pub cpu_iowait: Vec<f64>,
    pub cpu_steal: Vec<f64>,
    /// Disk I/O utilization (% of interval the busiest disk was busy).
    pub disk_util: Vec<f64>,
}

#[derive(serde::Deserialize)]
pub struct RangeQuery {
    #[serde(default)]
    pub range: Option<String>,
}

/// Maps a UI range key to a Postgres interval. Bounded by raw retention (1 day).
fn range_interval(range: &Option<String>) -> &'static str {
    match range.as_deref() {
        Some("30m") => "30 minutes",
        Some("3h") => "3 hours",
        Some("6h") => "6 hours",
        Some("12h") => "12 hours",
        Some("24h") => "24 hours",
        _ => "1 hour",
    }
}

/// GET /api/systems/:id/metrics?range=1h|6h|24h — samples for charting (newest last).
pub async fn system_metrics_series(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<MetricsHistory>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let interval = range_interval(&q.range);
    // mem/disk percentages are computed in SQL to keep the row tuple within
    // sqlx's 16-element FromRow limit.
    let sql = format!(
        "SELECT time, cpu_percent AS cpu, \
                CASE WHEN mem_total>0 THEN mem_used::float8/mem_total*100 ELSE 0 END AS mem_pct, \
                CASE WHEN disk_total>0 THEN disk_used::float8/disk_total*100 ELSE 0 END AS disk_pct, \
                net_rx, net_tx, COALESCE(disk_read,0) AS dread, COALESCE(disk_write,0) AS dwrite, \
                load1 AS l1, COALESCE(load5,0) AS l5, COALESCE(load15,0) AS l15, \
                COALESCE(cpu_user,0) AS cu, COALESCE(cpu_system,0) AS cs, \
                COALESCE(cpu_iowait,0) AS cio, COALESCE(cpu_steal,0) AS cst, \
                mem_used AS mu, COALESCE(disk_util,0) AS disk_util \
         FROM system_metrics WHERE system_id = $1 AND time > now() - interval '{interval}' \
         ORDER BY time ASC LIMIT 4000"
    );
    // FromRow struct (not a tuple) so we're not capped at sqlx's 16-column limit.
    #[derive(sqlx::FromRow)]
    struct Sample {
        time: chrono::DateTime<chrono::Utc>,
        cpu: f64,
        mem_pct: f64,
        disk_pct: f64,
        net_rx: i64,
        net_tx: i64,
        dread: i64,
        dwrite: i64,
        l1: f64,
        l5: f64,
        l15: f64,
        cu: f64,
        cs: f64,
        cio: f64,
        cst: f64,
        mu: i64,
        disk_util: f64,
    }
    let rows: Vec<Sample> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    let mut h = MetricsHistory {
        t: Vec::new(),
        cpu: Vec::new(),
        mem_pct: Vec::new(),
        mem_used: Vec::new(),
        disk_pct: Vec::new(),
        net_rx: Vec::new(),
        net_tx: Vec::new(),
        dr: Vec::new(),
        dw: Vec::new(),
        load1: Vec::new(),
        load5: Vec::new(),
        load15: Vec::new(),
        cpu_user: Vec::new(),
        cpu_system: Vec::new(),
        cpu_iowait: Vec::new(),
        cpu_steal: Vec::new(),
        disk_util: Vec::new(),
    };
    // Cumulative counters -> per-second rate from consecutive deltas.
    let mut prev: Option<(i64, i64, i64, i64, i64)> = None; // (ts, rx, tx, dr, dw)
    for row in rows {
        let Sample {
            time,
            cpu,
            mem_pct,
            disk_pct,
            net_rx,
            net_tx,
            dread,
            dwrite,
            l1,
            l5,
            l15,
            cu,
            cs,
            cio,
            cst,
            mu,
            disk_util,
        } = row;
        let ts = time.timestamp();
        h.t.push(ts);
        h.cpu.push(cpu);
        h.mem_pct.push(mem_pct);
        h.mem_used.push(mu as f64);
        h.disk_pct.push(disk_pct);
        h.load1.push(l1);
        h.load5.push(l5);
        h.load15.push(l15);
        h.cpu_user.push(cu);
        h.cpu_system.push(cs);
        h.cpu_iowait.push(cio);
        h.cpu_steal.push(cst);
        h.disk_util.push(disk_util);
        let (rx_rate, tx_rate, dr_rate, dw_rate) = match prev {
            Some((pt, prx, ptx, pdr, pdw)) if ts > pt => {
                let dt = (ts - pt) as f64;
                (
                    (net_rx - prx).max(0) as f64 / dt,
                    (net_tx - ptx).max(0) as f64 / dt,
                    (dread - pdr).max(0) as f64 / dt,
                    (dwrite - pdw).max(0) as f64 / dt,
                )
            }
            _ => (0.0, 0.0, 0.0, 0.0),
        };
        h.net_rx.push(rx_rate);
        h.net_tx.push(tx_rate);
        h.dr.push(dr_rate);
        h.dw.push(dw_rate);
        prev = Some((ts, net_rx, net_tx, dread, dwrite));
    }
    Ok(Json(h))
}

#[derive(Serialize)]
pub struct SystemRow {
    pub id: Uuid,
    pub name: String,
    pub hostname: Option<String>,
    pub kind: String,
    pub cluster: Option<String>,
    pub namespace: String,
    pub agent_version: Option<String>,
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

/// GET /api/systems — each server (in namespaces the caller can see) plus its
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
    )> = sqlx::query_as(
        "SELECT s.id, s.name, s.hostname, s.kind, s.cluster, n.name, s.agent_version, \
                s.kernel, s.cpu_model, s.cpu_cores, s.last_seen \
             FROM systems s JOIN namespaces n ON n.id = s.namespace_id \
             WHERE $1 OR s.namespace_id IN ( \
                SELECT namespace_id FROM memberships WHERE user_id = $2) \
             ORDER BY s.name",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    // Latest sample for ALL systems in ONE query (was N+1). DISTINCT ON + the
    // (system_id, time DESC) index makes this a fast per-system index scan.
    let ids: Vec<Uuid> = servers.iter().map(|s| s.0).collect();
    let latest_rows: Vec<(Uuid, f64, i64, i64, Option<i64>, Option<i64>, Option<f64>)> = sqlx::query_as(
        "SELECT DISTINCT ON (system_id) system_id, cpu_percent, mem_used, mem_total, disk_used, disk_total, disk_util \
         FROM system_metrics WHERE system_id = ANY($1) ORDER BY system_id, time DESC",
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
        namespace,
        agent_version,
        kernel,
        cpu_model,
        cpu_cores,
        last_seen,
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
            namespace,
            agent_version,
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

// ---- fleet overlay (NewRelic-style: one line per host across all systems) ----

#[derive(Serialize)]
pub struct FleetData {
    pub t: Vec<i64>,
    pub cpu: Vec<Series>,
    pub mem: Vec<Series>,
    pub disk: Vec<Series>,
    /// Network throughput rx+tx (bytes/sec).
    pub net: Vec<Series>,
}

fn range_bucket(range: &Option<String>) -> &'static str {
    match range.as_deref() {
        Some("6h") => "5 minutes",
        Some("12h") => "10 minutes",
        Some("24h") => "15 minutes",
        _ => "1 minute", // 30m / 1h / 3h
    }
}

/// GET /api/fleet?range= — per-host series for the fleet overlay charts. Bucketed
/// with time_bucket so many hosts stay light and share one timeline.
pub async fn fleet(
    State(state): State<AppState>,
    user: CurrentUser,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<FleetData>, StatusCode> {
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    let sys: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT s.id, s.name FROM systems s WHERE $1 OR s.namespace_id IN ( \
            SELECT namespace_id FROM memberships WHERE user_id = $2)",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    let names: HashMap<Uuid, String> = sys.into_iter().collect();

    let interval = range_interval(&q.range);
    let bucket = range_bucket(&q.range);
    let sql = format!(
        "SELECT system_id, time_bucket('{bucket}', time) AS b, \
                avg(cpu_percent) AS cpu, \
                avg(CASE WHEN mem_total>0 THEN mem_used::float8/mem_total*100 ELSE 0 END) AS mem, \
                avg(CASE WHEN disk_total>0 THEN disk_used::float8/disk_total*100 ELSE 0 END) AS disk, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx \
         FROM system_metrics WHERE time > now() - interval '{interval}' \
         GROUP BY system_id, b ORDER BY b"
    );
    let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, f64, f64, f64, i64, i64)> =
        sqlx::query_as(&sql)
            .fetch_all(&state.data)
            .await
            .map_err(internal)?;

    let mut times = BTreeSet::new();
    let mut cpu: BTreeMap<Uuid, HashMap<i64, f64>> = BTreeMap::new();
    let mut mem: BTreeMap<Uuid, HashMap<i64, f64>> = BTreeMap::new();
    let mut disk: BTreeMap<Uuid, HashMap<i64, f64>> = BTreeMap::new();
    let mut netc: BTreeMap<Uuid, BTreeMap<i64, i64>> = BTreeMap::new();
    for (sid, b, c, m, d, rx, tx) in rows {
        let ts = b.timestamp();
        times.insert(ts);
        cpu.entry(sid).or_default().insert(ts, c);
        mem.entry(sid).or_default().insert(ts, m);
        disk.entry(sid).or_default().insert(ts, d);
        netc.entry(sid).or_default().insert(ts, rx + tx);
    }
    let t: Vec<i64> = times.into_iter().collect();
    let mk = |map: BTreeMap<Uuid, HashMap<i64, f64>>| -> Vec<Series> {
        map.into_iter()
            .filter_map(|(sid, m)| {
                names.get(&sid).map(|name| Series {
                    name: name.clone(),
                    data: t.iter().map(|ts| m.get(ts).copied()).collect(),
                })
            })
            .collect()
    };
    let net: Vec<Series> = netc
        .into_iter()
        .filter_map(|(sid, bm)| {
            names.get(&sid).map(|name| {
                let mut prev: Option<(i64, i64)> = None;
                let data = t
                    .iter()
                    .map(|&ts| match bm.get(&ts) {
                        Some(&total) => {
                            let rate = match prev {
                                Some((pt, ptot)) if ts > pt => {
                                    Some((total - ptot).max(0) as f64 / (ts - pt) as f64)
                                }
                                _ => Some(0.0),
                            };
                            prev = Some((ts, total));
                            rate
                        }
                        None => None,
                    })
                    .collect();
                Series {
                    name: name.clone(),
                    data,
                }
            })
        })
        .collect();
    Ok(Json(FleetData {
        cpu: mk(cpu),
        mem: mk(mem),
        disk: mk(disk),
        net,
        t,
    }))
}

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
}

/// GET /api/monitors — each monitor (scoped to the caller's namespaces) plus
/// its latest heartbeat. Admins see every monitor.
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

    let mut rows = Vec::with_capacity(monitors.len());
    for (id, name, kind, target, namespace, interval_secs, enabled, config) in monitors {
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

        let (last_check, up, latency_ms, message) = match latest {
            Some((t, up, lat, msg)) => (Some(t), Some(up), lat, msg),
            None => (None, None, None, None),
        };
        rows.push(MonitorRow {
            id,
            name,
            kind,
            target,
            namespace,
            interval_secs,
            enabled,
            config: config.0,
            up,
            latency_ms,
            last_check,
            message,
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
    let (namespace, name, kind, target, interval_secs, enabled, config) =
        load_monitor(&state, &user, id).await?;

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
    let rows: Vec<(chrono::DateTime<chrono::Utc>, Option<f64>, Option<f64>)> =
        sqlx::query_as(&format!(
            "SELECT time_bucket('{bucket}', time) AS b, \
                avg(latency_ms)::float8 AS latency, \
                min((up)::int)::float8 AS up \
         FROM heartbeats WHERE monitor_id = $1 AND time > now() - interval '{interval}' \
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
pub struct Series {
    pub name: String,
    pub data: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub struct ContainersHistory {
    pub t: Vec<i64>,
    pub cpu: Vec<Series>,
    pub mem: Vec<Series>,
    /// Per-container network throughput (rx+tx bytes/sec).
    pub net: Vec<Series>,
}

/// Aligns per-key (ts -> value) maps onto a shared sorted timeline.
fn align(
    times: &[i64],
    per_key: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>>,
) -> Vec<Series> {
    per_key
        .into_iter()
        .map(|(name, m)| Series {
            name,
            data: times.iter().map(|t| m.get(t).copied()).collect(),
        })
        .collect()
}

/// GET /api/systems/:id/containers?range= — per-container CPU% and memory,
/// aligned onto one timeline for stacked charts.
pub async fn system_containers(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<ContainersHistory>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let interval = range_interval(&q.range);
    let sql = format!(
        "SELECT time, name, cpu_percent, mem_used, net_rx, net_tx FROM container_metrics \
         WHERE system_id = $1 AND time > now() - interval '{interval}' ORDER BY time ASC LIMIT 20000"
    );
    let rows: Vec<(chrono::DateTime<chrono::Utc>, String, f64, i64, i64, i64)> =
        sqlx::query_as(&sql)
            .bind(id)
            .fetch_all(&state.data)
            .await
            .map_err(internal)?;

    let mut times_set = std::collections::BTreeSet::new();
    let mut cpu_map: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        std::collections::BTreeMap::new();
    let mut mem_map: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        std::collections::BTreeMap::new();
    let mut net_map: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        std::collections::BTreeMap::new();
    // previous cumulative net total per container, for rate.
    let mut prev_net: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    for (time, name, cpu, mem, net_rx, net_tx) in rows {
        let ts = time.timestamp();
        times_set.insert(ts);
        cpu_map.entry(name.clone()).or_default().insert(ts, cpu);
        mem_map
            .entry(name.clone())
            .or_default()
            .insert(ts, mem as f64);
        let total = net_rx + net_tx;
        if let Some((pt, ptot)) = prev_net.get(&name) {
            if ts > *pt {
                let rate = (total - *ptot).max(0) as f64 / (ts - *pt) as f64;
                net_map.entry(name.clone()).or_default().insert(ts, rate);
            }
        }
        prev_net.insert(name, (ts, total));
    }
    let t: Vec<i64> = times_set.into_iter().collect();
    Ok(Json(ContainersHistory {
        cpu: align(&t, cpu_map),
        mem: align(&t, mem_map),
        net: align(&t, net_map),
        t,
    }))
}

#[derive(Serialize)]
pub struct TempsHistory {
    pub t: Vec<i64>,
    pub series: Vec<Series>,
}

/// GET /api/systems/:id/temps?range= — temperature sensors over time.
pub async fn system_temps(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<TempsHistory>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let interval = range_interval(&q.range);
    let sql = format!(
        "SELECT time, temps FROM system_metrics WHERE system_id = $1 AND temps IS NOT NULL \
         AND time > now() - interval '{interval}' ORDER BY time ASC LIMIT 2000"
    );
    let rows: Vec<(
        chrono::DateTime<chrono::Utc>,
        sqlx::types::Json<Vec<shared::TempReading>>,
    )> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    let mut times = Vec::new();
    let mut map: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        std::collections::BTreeMap::new();
    for (time, temps) in rows {
        let ts = time.timestamp();
        times.push(ts);
        for r in temps.0 {
            map.entry(r.label).or_default().insert(ts, r.celsius as f64);
        }
    }
    Ok(Json(TempsHistory {
        series: align(&times, map),
        t: times,
    }))
}

#[derive(Serialize)]
pub struct GpuHistory {
    pub t: Vec<i64>,
    pub usage: Vec<Series>,
    pub vram: Vec<Series>,
    pub power: Vec<Series>,
}

/// GET /api/systems/:id/gpu?range= — per-GPU utilization / VRAM% / power.
pub async fn system_gpu(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<GpuHistory>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let interval = range_interval(&q.range);
    let sql = format!(
        "SELECT time, gpus FROM system_metrics WHERE system_id = $1 AND gpus IS NOT NULL \
         AND gpus <> '[]'::jsonb AND time > now() - interval '{interval}' ORDER BY time ASC LIMIT 2000"
    );
    let rows: Vec<(
        chrono::DateTime<chrono::Utc>,
        sqlx::types::Json<Vec<shared::GpuStat>>,
    )> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    let mut times = Vec::new();
    let mut usage: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        Default::default();
    let mut vram: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        Default::default();
    let mut power: std::collections::BTreeMap<String, std::collections::HashMap<i64, f64>> =
        Default::default();
    for (time, gpus) in rows {
        let ts = time.timestamp();
        times.push(ts);
        for g in gpus.0 {
            usage
                .entry(g.name.clone())
                .or_default()
                .insert(ts, g.usage_percent as f64);
            let vp = if g.mem_total > 0 {
                g.mem_used as f64 / g.mem_total as f64 * 100.0
            } else {
                0.0
            };
            vram.entry(g.name.clone()).or_default().insert(ts, vp);
            power
                .entry(g.name)
                .or_default()
                .insert(ts, g.power_w as f64);
        }
    }
    Ok(Json(GpuHistory {
        usage: align(&times, usage),
        vram: align(&times, vram),
        power: align(&times, power),
        t: times,
    }))
}

fn internal<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "web handler DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}
