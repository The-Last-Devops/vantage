//! The rollup ladder for every metric family: raw → 1m → 1h.
//!
//! Rollups are plain tables filled by this job, not TimescaleDB continuous aggregates.
//! A CAgg cannot be `ALTER`ed, so adding one metric column forces a drop and rebuild of
//! the whole chain, after which the long tier can only refill from whatever the shorter
//! tier still holds. That is not hypothetical: it is why `system_metrics_1h` carried 43
//! days of history under a 365-day policy. A plain table takes `ADD COLUMN` and keeps
//! everything it already has.
//!
//! Two aggregation shapes, because the two families are shaped differently:
//!
//! * **host** — one row per system per sample, so a bucket is a straight `avg`/`max`.
//! * **k8s** — one row per *container* per sample. A chart wants the cluster (or
//!   namespace, or workload) TOTAL over time, which is the average across snapshots of
//!   the sum across containers. Writing `avg(cpu_millicores)` instead yields one
//!   container's average: a smooth, believable line, wrong by however many containers
//!   are running. Hence the inner `GROUP BY` on the sample timestamp.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use std::time::Duration;

/// How often the job runs — the finest rollup bucket, so a closed minute is picked up on
/// the next pass.
const TICK: Duration = Duration::from_secs(60);

/// One tier to fill: where it reads from, what it writes to, and how far back to start
/// the very first time it runs.
struct Tier {
    /// Key in `rollup_progress`.
    name: &'static str,
    /// Longest span one pass will fill. Without a cap the first pass after a deploy tries
    /// to aggregate every row at once; with it, a backlog drains over a few minutes.
    max_span_hours: i64,
    /// Where a tier starts when it has never been filled. There is nothing older to
    /// recover than the source tier's own retention, so this just needs to cover it.
    cold_start_hours: i64,
}

const TIERS: &[Tier] = &[
    // Source is raw at the host cadence (5s) and kept 8h, so a 6h window is one pass.
    Tier {
        name: "system_1m",
        max_span_hours: 6,
        cold_start_hours: 8,
    },
    // Sources are the 1m tiers, kept 2 days.
    Tier {
        name: "system_1h",
        max_span_hours: 24,
        cold_start_hours: 48,
    },
    Tier {
        name: "kube_1h",
        max_span_hours: 24,
        cold_start_hours: 48,
    },
];

/// Host columns that are averaged over a bucket.
const HOST_AVG: &[&str] = &[
    "cpu_percent",
    "mem_used",
    "mem_total",
    "swap_used",
    "swap_total",
    "disk_used",
    "disk_total",
    "load1",
    "load5",
    "load15",
    "cpu_user",
    "cpu_system",
    "cpu_iowait",
    "cpu_steal",
    "disk_util",
    "mem_available",
    "mem_buffers",
    "mem_cached",
    "mem_free",
];
/// Host columns that are cumulative counters — a max over the bucket stays a max when
/// one tier rolls up into the next.
const HOST_MAX: &[&str] = &["net_rx", "net_tx", "disk_read", "disk_write"];

pub fn spawn(data: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            run_once(&data).await;
        }
    });
}

/// One pass over every tier, finest first so a coarse tier never runs ahead of its
/// source. Best-effort: a failure leaves the progress marker untouched, so the next pass
/// retries the same window instead of skipping it.
pub async fn run_once(data: &PgPool) {
    for tier in TIERS {
        let r = match tier.name {
            "system_1m" => fill_host(data, tier, "system_metrics_5s", "time", "1 minute").await,
            "system_1h" => fill_host(data, tier, "system_metrics_1m", "bucket", "1 hour").await,
            "kube_1h" => fill_kube(data, tier).await,
            _ => Ok(()),
        };
        if let Err(e) = r {
            tracing::warn!(error = %e, tier = tier.name, "rollup fill failed");
        }
    }
}

/// How far a tier has been filled (exclusive). Absent → its cold start.
async fn progress(data: &PgPool, tier: &Tier) -> DateTime<Utc> {
    let row: Option<(DateTime<Utc>,)> =
        sqlx::query_as("SELECT filled_to FROM rollup_progress WHERE tier = $1")
            .bind(tier.name)
            .fetch_optional(data)
            .await
            .ok()
            .flatten();
    match row {
        Some((t,)) => t,
        None => Utc::now() - ChronoDuration::hours(tier.cold_start_hours),
    }
}

async fn set_progress(data: &PgPool, tier: &Tier, to: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO rollup_progress (tier, filled_to) VALUES ($1, $2) \
         ON CONFLICT (tier) DO UPDATE SET filled_to = EXCLUDED.filled_to",
    )
    .bind(tier.name)
    .bind(to)
    .execute(data)
    .await
    .map(|_| ())
}

/// The window this pass should fill: from where we stopped, up to the last CLOSED bucket,
/// capped at the tier's max span. Stopping at a closed bucket matters — rolling up the
/// bucket still being written stores a partial average, and `ON CONFLICT DO NOTHING` then
/// keeps that wrong value forever.
async fn window(
    data: &PgPool,
    tier: &Tier,
    bucket: &str,
    ceiling: Option<DateTime<Utc>>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, sqlx::Error> {
    let from = progress(data, tier).await;
    let span = tier.max_span_hours;
    let (to,): (DateTime<Utc>,) = sqlx::query_as(&format!(
        "SELECT least(time_bucket(INTERVAL '{bucket}', coalesce($2::timestamptz, now())), \
                      $1::timestamptz + INTERVAL '{span} hours')"
    ))
    .bind(from)
    .bind(ceiling)
    .fetch_one(data)
    .await?;
    Ok(if to <= from { None } else { Some((from, to)) })
}

async fn fill_host(
    data: &PgPool,
    tier: &Tier,
    src: &str,
    srccol: &str,
    bucket: &str,
) -> Result<(), sqlx::Error> {
    // The 1h tier must never pass what the 1m tier has actually filled, or an hour whose
    // later minutes are not rolled up yet is stored short and then frozen by ON CONFLICT.
    let ceiling = if src.ends_with("_1m") {
        Some(progress(data, &TIERS[0]).await)
    } else {
        None
    };
    let Some((from, to)) = window(data, tier, bucket, ceiling).await? else {
        return Ok(());
    };
    let cols: Vec<&str> = HOST_AVG.iter().chain(HOST_MAX.iter()).copied().collect();
    let aggs: Vec<String> = HOST_AVG
        .iter()
        .map(|c| format!("avg({c})::float8"))
        .chain(HOST_MAX.iter().map(|c| format!("max({c})")))
        .collect();
    let dest = if bucket == "1 minute" {
        "system_metrics_1m"
    } else {
        "system_metrics_1h"
    };
    sqlx::query(&format!(
        "INSERT INTO {dest} (bucket, system_id, {}) \
         SELECT time_bucket(INTERVAL '{bucket}', {srccol}), system_id, {} \
         FROM {src} WHERE {srccol} >= $1 AND {srccol} < $2 \
         GROUP BY 1, 2 \
         ON CONFLICT DO NOTHING",
        cols.join(", "),
        aggs.join(", ")
    ))
    .bind(from)
    .bind(to)
    .execute(data)
    .await?;
    set_progress(data, tier, to).await
}

/// k8s 1m (per container) → 1h, grouped by namespace / workload / node.
///
/// `pod` and `labels` are deliberately dropped: pod names change on every rollout, so
/// keeping them is what makes the detail tier unaffordable past a couple of days. Label
/// filtering therefore only works inside the 1m tier's window.
async fn fill_kube(data: &PgPool, tier: &Tier) -> Result<(), sqlx::Error> {
    let Some((from, to)) = window(data, tier, "1 hour", None).await? else {
        return Ok(());
    };
    sqlx::query(
        "INSERT INTO kube_metrics_1h \
           (bucket, system_id, namespace, workload, workload_kind, node, \
            cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts) \
         SELECT b, system_id, namespace, workload, workload_kind, node, \
                avg(cpu), max(cpu), avg(mem), max(mem), avg(n), max(restarts) \
         FROM ( \
           SELECT time_bucket(INTERVAL '1 hour', time) AS b, time, system_id, \
                  namespace, workload, workload_kind, node, \
                  sum(cpu_millicores)::float8 AS cpu, sum(mem_bytes)::float8 AS mem, \
                  count(*)::float8 AS n, max(restarts) AS restarts \
           FROM kube_metrics_1m \
           WHERE time >= $1 AND time < $2 \
           GROUP BY 1, 2, 3, 4, 5, 6, 7 \
         ) s \
         GROUP BY 1, 2, 3, 4, 5, 6 \
         ON CONFLICT DO NOTHING",
    )
    .bind(from)
    .bind(to)
    .execute(data)
    .await?;
    set_progress(data, tier, to).await
}
