use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use uuid::Uuid;

use super::Series;
use crate::auth::CurrentUser;
use crate::web::{can_view_system, chart_tier, internal, RangeQuery};
use crate::AppState;

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
    let (_, _, interval, _) = chart_tier(&q.range);
    let sql = format!(
        "SELECT time, name, cpu_percent, mem_used, net_rx, net_tx FROM container_metrics_5s \
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
    let (_, _, interval, _) = chart_tier(&q.range);
    let sql = format!(
        "SELECT time, temps FROM system_metrics_5s WHERE system_id = $1 AND temps IS NOT NULL \
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
    let (_, _, interval, _) = chart_tier(&q.range);
    let sql = format!(
        "SELECT time, gpus FROM system_metrics_5s WHERE system_id = $1 AND gpus IS NOT NULL \
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
