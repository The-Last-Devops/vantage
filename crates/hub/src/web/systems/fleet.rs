use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use uuid::Uuid;

use super::Series;
use crate::auth::CurrentUser;
use crate::web::{chart_tier, internal, RangeQuery};
use crate::AppState;

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

/// GET /api/fleet?range= — per-host series for the fleet overlay charts. Bucketed
/// with time_bucket so many hosts stay light and share one timeline.
pub async fn fleet(
    State(state): State<AppState>,
    user: CurrentUser,
    axum::extract::Query(q): axum::extract::Query<RangeQuery>,
) -> Result<Json<FleetData>, StatusCode> {
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    let sys: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT s.id, s.name FROM systems s WHERE $1 OR s.workspace_id IN ( \
            SELECT workspace_id FROM memberships WHERE user_id = $2)",
    )
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    let names: HashMap<Uuid, String> = sys.into_iter().collect();

    let (suffix, timecol, interval, bucket) = chart_tier(&q.range);
    let sql = format!(
        "SELECT system_id, time_bucket('{bucket}', {timecol}) AS b, \
                avg(cpu_percent) AS cpu, \
                avg(CASE WHEN mem_total>0 THEN mem_used::float8/mem_total*100 ELSE 0 END) AS mem, \
                avg(CASE WHEN disk_total>0 THEN disk_used::float8/disk_total*100 ELSE 0 END) AS disk, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx \
         FROM system_metrics{suffix} WHERE {timecol} > now() - interval '{interval}' \
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
