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

/// True if the user may view the given server (admin, or member of its namespace).
pub async fn can_view_system(
    state: &AppState,
    user: &CurrentUser,
    system_id: Uuid,
) -> Result<bool, StatusCode> {
    if user.is_admin {
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
    /// Per-core CPU usage % (one Series per core, htop-style).
    pub cores: Vec<Series>,
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
    let sql = format!(
        "SELECT time, cpu_percent, mem_used, mem_total, disk_used, disk_total, net_rx, net_tx, \
                COALESCE(disk_read,0), COALESCE(disk_write,0), load1, COALESCE(load5,0), COALESCE(load15,0), cpu_per_core \
         FROM system_metrics WHERE system_id = $1 AND time > now() - interval '{interval}' \
         ORDER BY time ASC LIMIT 4000"
    );
    type Sample = (
        chrono::DateTime<chrono::Utc>,
        f64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        f64,
        f64,
        f64,
        Option<sqlx::types::Json<Vec<f32>>>,
    );
    let rows: Vec<Sample> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    let pct = |used: i64, total: i64| {
        if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        }
    };
    let ncores = rows
        .iter()
        .map(|r| r.13.as_ref().map(|j| j.0.len()).unwrap_or(0))
        .max()
        .unwrap_or(0);
    let mut core_data: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(rows.len()); ncores];

    let mut h = MetricsHistory {
        t: Vec::new(),
        cpu: Vec::new(),
        mem_pct: Vec::new(),
        disk_pct: Vec::new(),
        net_rx: Vec::new(),
        net_tx: Vec::new(),
        dr: Vec::new(),
        dw: Vec::new(),
        load1: Vec::new(),
        load5: Vec::new(),
        load15: Vec::new(),
        cores: Vec::new(),
    };
    // Cumulative counters -> per-second rate from consecutive deltas.
    let mut prev: Option<(i64, i64, i64, i64, i64)> = None; // (ts, rx, tx, dr, dw)
    for row in rows {
        let (
            time,
            cpu,
            mem_used,
            mem_total,
            disk_used,
            disk_total,
            net_rx,
            net_tx,
            dread,
            dwrite,
            l1,
            l5,
            l15,
            pc,
        ) = row;
        let ts = time.timestamp();
        h.t.push(ts);
        h.cpu.push(cpu);
        h.mem_pct.push(pct(mem_used, mem_total));
        h.disk_pct.push(pct(disk_used, disk_total));
        h.load1.push(l1);
        h.load5.push(l5);
        h.load15.push(l15);
        let per_core = pc.map(|j| j.0).unwrap_or_default();
        for (i, slot) in core_data.iter_mut().enumerate() {
            slot.push(per_core.get(i).map(|v| *v as f64));
        }
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
    h.cores = core_data
        .into_iter()
        .enumerate()
        .map(|(i, data)| Series {
            name: format!("core {i}"),
            data,
        })
        .collect();
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
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub cpu_percent: Option<f64>,
    pub mem_used: Option<i64>,
    pub mem_total: Option<i64>,
    pub disk_used: Option<i64>,
    pub disk_total: Option<i64>,
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
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT s.id, s.name, s.hostname, s.kind, s.cluster, n.name, s.agent_version, s.last_seen \
             FROM systems s JOIN namespaces n ON n.id = s.namespace_id \
             WHERE $1 OR s.namespace_id IN ( \
                SELECT namespace_id FROM memberships WHERE user_id = $2) \
             ORDER BY s.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    // Latest sample for ALL systems in ONE query (was N+1). DISTINCT ON + the
    // (system_id, time DESC) index makes this a fast per-system index scan.
    let ids: Vec<Uuid> = servers.iter().map(|s| s.0).collect();
    let latest_rows: Vec<(Uuid, f64, i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT DISTINCT ON (system_id) system_id, cpu_percent, mem_used, mem_total, disk_used, disk_total \
         FROM system_metrics WHERE system_id = ANY($1) ORDER BY system_id, time DESC",
    )
    .bind(&ids)
    .fetch_all(&state.data)
    .await
    .map_err(internal)?;
    let mut latest: std::collections::HashMap<Uuid, (f64, i64, i64, Option<i64>, Option<i64>)> =
        std::collections::HashMap::with_capacity(latest_rows.len());
    for (sid, c, mu, mt, du, dt) in latest_rows {
        latest.insert(sid, (c, mu, mt, du, dt));
    }

    let mut rows = Vec::with_capacity(servers.len());
    for (id, name, hostname, kind, cluster, namespace, agent_version, last_seen) in servers {
        let (cpu_percent, mem_used, mem_total, disk_used, disk_total) = match latest.get(&id) {
            Some(&(c, u, t, du, dt)) => (Some(c), Some(u), Some(t), du, dt),
            None => (None, None, None, None, None),
        };
        rows.push(SystemRow {
            id,
            name,
            hostname,
            kind,
            cluster,
            namespace,
            agent_version,
            last_seen,
            cpu_percent,
            mem_used,
            mem_total,
            disk_used,
            disk_total,
        });
    }
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct MonitorRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub target: String,
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
    let monitors: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT m.id, m.name, m.kind::text, m.target FROM monitors m \
         WHERE $1 OR m.namespace_id IN ( \
            SELECT namespace_id FROM memberships WHERE user_id = $2) \
         ORDER BY m.name",
    )
    .bind(user.is_admin)
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    let mut rows = Vec::with_capacity(monitors.len());
    for (id, name, kind, target) in monitors {
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
            up,
            latency_ms,
            last_check,
            message,
        });
    }
    Ok(Json(rows))
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
