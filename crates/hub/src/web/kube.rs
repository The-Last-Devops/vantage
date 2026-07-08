//! Read API for Kubernetes per-container stats (`kube_container_stats`).
//!
//! The agent stores the granular unit — one row per container per snapshot, with
//! pod metadata + labels — and these endpoints **aggregate on read** by whatever
//! dimension the UI asks for (namespace / workload / label), plus a raw drill-down
//! list and a time series for charts. All are scoped by `can_view_system`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::web::{can_view_system, chart_tier, internal};
use crate::AppState;

#[derive(Deserialize)]
pub struct AggQuery {
    /// Grouping dimension: "namespace" (default) | "workload" | "label".
    pub by: Option<String>,
    /// Label key to group by when `by=label` (e.g. "app").
    pub label: Option<String>,
    /// Optional namespace to scope the whole aggregate to (disambiguates workloads
    /// that share a name across namespaces).
    pub ns: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AggRow {
    /// The group value (namespace name, "Kind/name" workload, or label value).
    #[sqlx(rename = "grp")]
    #[serde(rename = "group")]
    pub grp: Option<String>,
    /// The namespace this group belongs to — set for `by=workload` (so same-named
    /// workloads in different namespaces stay distinct); null otherwise.
    pub namespace: Option<String>,
    pub cpu_millicores: f64,
    pub mem_bytes: f64,
    pub pods: i64,
    pub containers: i64,
    pub restarts: i64,
}

#[derive(Serialize)]
pub struct AggResponse {
    /// Snapshot time (epoch seconds) the totals are computed from; null if no data.
    pub as_of: Option<i64>,
    pub by: String,
    pub groups: Vec<AggRow>,
}

/// GET /api/systems/:id/kube/aggregate?by=namespace|workload|label&label=<key>
/// — latest-snapshot usage grouped by the chosen dimension.
pub async fn kube_aggregate(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<AggQuery>,
) -> Result<Json<AggResponse>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let by = q.by.as_deref().unwrap_or("namespace");
    if !matches!(by, "namespace" | "workload" | "label") {
        return Err(StatusCode::BAD_REQUEST);
    }
    if by == "label" && q.label.as_deref().unwrap_or("").is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let as_of: Option<(Option<chrono::DateTime<chrono::Utc>>,)> =
        sqlx::query_as("SELECT max(time) FROM kube_container_stats WHERE system_id = $1")
            .bind(id)
            .fetch_optional(&state.data)
            .await
            .map_err(internal)?;
    let as_of_ts = as_of.and_then(|r| r.0).map(|t| t.timestamp());

    // Built with QueryBuilder so the label key + namespace filter are bound, never
    // interpolated. Grouping columns come from a fixed allowlist above.
    let mut qb = sqlx::QueryBuilder::new(
        "WITH latest AS (SELECT max(time) AS t FROM kube_container_stats WHERE system_id = ",
    );
    qb.push_bind(id).push(") SELECT ");
    match by {
        "namespace" => {
            qb.push("namespace AS grp, NULL::text AS namespace");
        }
        "workload" => {
            // Keep same-named workloads in different namespaces distinct.
            qb.push("(CASE WHEN workload = '' THEN '—' ELSE workload_kind || '/' || workload END) AS grp, namespace AS namespace");
        }
        _ => {
            qb.push("COALESCE(labels ->> ")
                .push_bind(q.label.clone().unwrap_or_default())
                .push(", '—') AS grp, NULL::text AS namespace");
        }
    }
    qb.push(
        ", sum(cpu_millicores)::float8 AS cpu_millicores, sum(mem_bytes)::float8 AS mem_bytes, \
         count(DISTINCT pod)::int8 AS pods, count(*)::int8 AS containers, sum(restarts)::int8 AS restarts \
         FROM kube_container_stats c, latest WHERE c.system_id = ",
    );
    qb.push_bind(id).push(" AND c.time = latest.t");
    if let Some(ns) = q.ns.filter(|s| !s.is_empty()) {
        qb.push(" AND namespace = ").push_bind(ns);
    }
    // Group by the display value; workload also groups by namespace so it isn't merged.
    match by {
        "workload" => qb.push(" GROUP BY namespace, grp"),
        _ => qb.push(" GROUP BY grp"),
    };
    qb.push(" ORDER BY cpu_millicores DESC NULLS LAST LIMIT 500");
    let groups = qb
        .build_query_as::<AggRow>()
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    Ok(Json(AggResponse {
        as_of: as_of_ts,
        by: by.to_string(),
        groups,
    }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct KubeSummary {
    /// Snapshot time (epoch seconds) the totals are computed from; null if no data.
    pub as_of: Option<i64>,
    pub cpu_millicores: f64,
    pub mem_bytes: f64,
    pub pods: i64,
    pub pods_running: i64,
    pub containers: i64,
    /// Cumulative container restart count across the cluster (not a 24h delta).
    pub restarts: i64,
    pub namespaces: i64,
    pub nodes: i64,
}

/// GET /api/systems/:id/kube/summary — latest-snapshot cluster roll-up (used by the
/// Cluster page's KPI strip and the Clusters list cards).
pub async fn kube_summary(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<KubeSummary>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let row: KubeSummary = sqlx::query_as(
        "WITH latest AS (SELECT max(time) AS t FROM kube_container_stats WHERE system_id = $1) \
         SELECT extract(epoch FROM max(c.time))::int8 AS as_of, \
                COALESCE(sum(cpu_millicores),0)::float8 AS cpu_millicores, \
                COALESCE(sum(mem_bytes),0)::float8 AS mem_bytes, \
                count(DISTINCT pod)::int8 AS pods, \
                count(DISTINCT pod) FILTER (WHERE phase = 'Running')::int8 AS pods_running, \
                count(*)::int8 AS containers, \
                COALESCE(sum(restarts),0)::int8 AS restarts, \
                count(DISTINCT namespace)::int8 AS namespaces, \
                count(DISTINCT node) FILTER (WHERE node <> '')::int8 AS nodes \
         FROM kube_container_stats c, latest WHERE c.system_id = $1 AND c.time = latest.t",
    )
    .bind(id)
    .fetch_one(&state.data)
    .await
    .map_err(internal)?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct KubeFilter {
    pub ns: Option<String>,
    pub workload: Option<String>,
    /// Label key + value to filter by (both required together), e.g. lk=app&lv=web.
    pub lk: Option<String>,
    pub lv: Option<String>,
    pub range: Option<String>,
}

/// Push the optional ns / workload / label filters shared by the list + series
/// queries. Values are bound (never interpolated).
fn push_filters(qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>, f: &KubeFilter) {
    if let Some(ns) = f.ns.clone().filter(|s| !s.is_empty()) {
        qb.push(" AND namespace = ").push_bind(ns);
    }
    if let Some(w) = f.workload.clone().filter(|s| !s.is_empty()) {
        qb.push(" AND workload = ").push_bind(w);
    }
    if let (Some(k), Some(v)) = (
        f.lk.clone().filter(|s| !s.is_empty()),
        f.lv.clone().filter(|s| !s.is_empty()),
    ) {
        qb.push(" AND labels ->> ")
            .push_bind(k)
            .push(" = ")
            .push_bind(v);
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ContainerRow {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub node: String,
    pub phase: String,
    pub workload: String,
    pub workload_kind: String,
    pub cpu_millicores: i64,
    pub mem_bytes: i64,
    pub restarts: i32,
    pub labels: serde_json::Value,
}

/// GET /api/systems/:id/kube/containers?ns=&workload= — latest-snapshot container
/// rows (with pod metadata + labels) for drill-down, optionally filtered.
pub async fn kube_containers(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(f): Query<KubeFilter>,
) -> Result<Json<Vec<ContainerRow>>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut qb = sqlx::QueryBuilder::new(
        "WITH latest AS (SELECT max(time) AS t FROM kube_container_stats WHERE system_id = ",
    );
    qb.push_bind(id).push(
        ") SELECT namespace, pod, container, node, phase, workload, workload_kind, \
              cpu_millicores, mem_bytes, restarts, labels \
         FROM kube_container_stats c, latest WHERE c.system_id = ",
    );
    qb.push_bind(id).push(" AND c.time = latest.t");
    push_filters(&mut qb, &f);
    qb.push(" ORDER BY namespace, pod, container LIMIT 2000");
    let rows = qb
        .build_query_as::<ContainerRow>()
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct KubeSeries {
    pub t: Vec<i64>,
    /// Total CPU across matching containers (millicores), averaged per bucket.
    pub cpu_millicores: Vec<f64>,
    /// Total memory across matching containers (bytes), averaged per bucket.
    pub mem_bytes: Vec<f64>,
}

/// GET /api/systems/:id/kube/series?ns=&workload=&range= — CPU/mem totals over
/// time for charting. Sums per snapshot, then averages per display bucket (so
/// multiple snapshots in one bucket don't double-count).
pub async fn kube_series(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(f): Query<KubeFilter>,
) -> Result<Json<KubeSeries>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    // Window + bucket come from the shared allowlist (constants, safe to inline).
    let (_suffix, _timecol, interval, bucket) = chart_tier(&f.range);
    let mut qb = sqlx::QueryBuilder::new(format!(
        "SELECT time_bucket('{bucket}', time) AS t, avg(scpu)::float8 AS cpu, avg(smem)::float8 AS mem FROM ( \
             SELECT time, sum(cpu_millicores) AS scpu, sum(mem_bytes) AS smem \
             FROM kube_container_stats WHERE system_id = "
    ));
    qb.push_bind(id)
        .push(format!(" AND time > now() - interval '{interval}'"));
    push_filters(&mut qb, &f);
    qb.push(" GROUP BY time) s GROUP BY 1 ORDER BY 1 LIMIT 4000");

    #[derive(sqlx::FromRow)]
    struct Row {
        t: chrono::DateTime<chrono::Utc>,
        cpu: f64,
        mem: f64,
    }
    let rows = qb
        .build_query_as::<Row>()
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;
    let mut out = KubeSeries {
        t: Vec::with_capacity(rows.len()),
        cpu_millicores: Vec::with_capacity(rows.len()),
        mem_bytes: Vec::with_capacity(rows.len()),
    };
    for r in rows {
        out.t.push(r.t.timestamp());
        out.cpu_millicores.push(r.cpu);
        out.mem_bytes.push(r.mem);
    }
    Ok(Json(out))
}

/// Cap on overlaid series so a big cluster doesn't render hundreds of lines.
const SERIES_BY_MAX_GROUPS: usize = 16;

#[derive(Serialize)]
pub struct GroupSeries {
    pub name: String,
    /// CPU (millicores) per bucket, aligned to `t`; null where the group had no data.
    pub cpu_millicores: Vec<Option<f64>>,
    /// Memory (bytes) per bucket, aligned to `t`.
    pub mem_bytes: Vec<Option<f64>>,
}

#[derive(Serialize)]
pub struct SeriesByResponse {
    pub t: Vec<i64>,
    /// True if some low-usage groups were dropped to the top-N cap.
    pub truncated: bool,
    pub groups: Vec<GroupSeries>,
}

/// GET /api/systems/:id/kube/series-by?by=namespace|workload|label&label=&ns=&range=
/// — one time series **per group** (overlaid on the chart, like the fleet overlay).
/// Bounded to the top-N groups by total usage so a large cluster stays readable.
pub async fn kube_series_by(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(q): Query<AggQuery>,
    Query(f): Query<KubeFilter>,
) -> Result<Json<SeriesByResponse>, StatusCode> {
    if !can_view_system(&state, &user, id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let by = q.by.as_deref().unwrap_or("namespace");
    if !matches!(by, "namespace" | "workload" | "label") {
        return Err(StatusCode::BAD_REQUEST);
    }
    if by == "label" && q.label.as_deref().unwrap_or("").is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (_s, _tc, interval, bucket) = chart_tier(&f.range);

    // Per (bucket, group): sum per snapshot, then average across snapshots in the bucket.
    // bucket/interval are constants from chart_tier's allowlist — safe to inline (a bound
    // text param can't stand in for time_bucket's INTERVAL argument).
    let mut qb = sqlx::QueryBuilder::new(format!(
        "SELECT tb, grp, avg(scpu)::float8 AS cpu, avg(smem)::float8 AS mem FROM ( \
         SELECT time_bucket('{bucket}', time) AS tb, time, "
    ));
    match by {
        "namespace" => {
            qb.push("namespace AS grp");
        }
        "workload" => {
            // Qualify with namespace so same-named workloads are separate lines.
            qb.push("(namespace || ' · ' || workload_kind || '/' || workload) AS grp");
        }
        _ => {
            qb.push("COALESCE(labels ->> ")
                .push_bind(q.label.clone().unwrap_or_default())
                .push(", '—') AS grp");
        }
    }
    qb.push(", sum(cpu_millicores) AS scpu, sum(mem_bytes) AS smem FROM kube_container_stats WHERE system_id = ");
    qb.push_bind(id)
        .push(format!(" AND time > now() - interval '{interval}'"));
    if let Some(ns) = q.ns.filter(|s| !s.is_empty()) {
        qb.push(" AND namespace = ").push_bind(ns);
    }
    qb.push(" GROUP BY tb, time, grp) s GROUP BY tb, grp ORDER BY tb LIMIT 20000");

    #[derive(sqlx::FromRow)]
    struct Row {
        tb: chrono::DateTime<chrono::Utc>,
        grp: Option<String>,
        cpu: f64,
        mem: f64,
    }
    let rows = qb
        .build_query_as::<Row>()
        .fetch_all(&state.data)
        .await
        .map_err(internal)?;

    // Distinct sorted bucket axis + its index.
    let mut times: Vec<i64> = rows.iter().map(|r| r.tb.timestamp()).collect();
    times.sort_unstable();
    times.dedup();
    let idx: std::collections::HashMap<i64, usize> =
        times.iter().enumerate().map(|(i, &t)| (t, i)).collect();
    let n = times.len();

    // Accumulate per group, tracking total for top-N ranking.
    struct Acc {
        cpu: Vec<Option<f64>>,
        mem: Vec<Option<f64>>,
        total: f64,
    }
    let mut groups: std::collections::HashMap<String, Acc> = std::collections::HashMap::new();
    for r in &rows {
        let name = r.grp.clone().unwrap_or_else(|| "—".into());
        let a = groups.entry(name).or_insert_with(|| Acc {
            cpu: vec![None; n],
            mem: vec![None; n],
            total: 0.0,
        });
        if let Some(&i) = idx.get(&r.tb.timestamp()) {
            a.cpu[i] = Some(r.cpu);
            a.mem[i] = Some(r.mem);
            a.total += r.cpu;
        }
    }
    let mut ranked: Vec<(String, Acc)> = groups.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.total
            .partial_cmp(&a.1.total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let truncated = ranked.len() > SERIES_BY_MAX_GROUPS;
    ranked.truncate(SERIES_BY_MAX_GROUPS);

    Ok(Json(SeriesByResponse {
        t: times,
        truncated,
        groups: ranked
            .into_iter()
            .map(|(name, a)| GroupSeries {
                name,
                cpu_millicores: a.cpu,
                mem_bytes: a.mem,
            })
            .collect(),
    }))
}
