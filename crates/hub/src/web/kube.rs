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
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AggRow {
    /// The group value (namespace name, "Kind/name" workload, or label value).
    #[sqlx(rename = "grp")]
    #[serde(rename = "group")]
    pub grp: Option<String>,
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
    // Grouping expressions are chosen from a fixed allowlist — never interpolated
    // from user input. The label VALUE is bound ($2), not interpolated.
    let grp_expr = match by {
        "namespace" => "namespace",
        "workload" => "CASE WHEN workload = '' THEN '—' ELSE workload_kind || '/' || workload END",
        "label" => "COALESCE(labels ->> $2, '—')",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
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

    let sql = format!(
        "WITH latest AS (SELECT max(time) AS t FROM kube_container_stats WHERE system_id = $1) \
         SELECT {grp_expr} AS grp, \
                sum(cpu_millicores)::float8 AS cpu_millicores, \
                sum(mem_bytes)::float8 AS mem_bytes, \
                count(DISTINCT pod)::int8 AS pods, \
                count(*)::int8 AS containers, \
                sum(restarts)::int8 AS restarts \
         FROM kube_container_stats c, latest \
         WHERE c.system_id = $1 AND c.time = latest.t \
         GROUP BY grp ORDER BY cpu_millicores DESC NULLS LAST LIMIT 500"
    );
    let mut query = sqlx::query_as::<_, AggRow>(&sql).bind(id);
    if by == "label" {
        query = query.bind(q.label.unwrap_or_default());
    }
    let groups = query.fetch_all(&state.data).await.map_err(internal)?;

    Ok(Json(AggResponse {
        as_of: as_of_ts,
        by: by.to_string(),
        groups,
    }))
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
