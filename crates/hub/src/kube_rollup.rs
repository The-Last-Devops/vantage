//! K8s rollup ladder: raw `kube_container_stats` → `kube_rollup_5m` → `kube_rollup_1h`.
//!
//! Host metrics downsample through TimescaleDB continuous aggregates; k8s deliberately
//! does not. A CAgg cannot be `ALTER`ed, so adding one metric column forces a drop and
//! rebuild of the whole chain, after which the long tier can only refill from whatever
//! the shorter tier still holds — which is why `system_metrics_1h` carried ~44 days of
//! history under a 365-day policy. These tiers are plain tables filled by this job, so
//! a new column is an `ADD COLUMN` and the history survives.
//!
//! Correctness note: a chart wants, per bucket, the **average across snapshots of the
//! sum across containers** — not the average of individual container samples. So every
//! fill is two-stage: group by snapshot first (summing containers), then aggregate those
//! per-snapshot totals over the bucket. This also keeps the ladder composable: the 1h
//! tier averages the 5m averages (equal-width buckets) and takes the max of the 5m
//! maxima, so `cpu_max` stays a true peak snapshot no matter how coarse the tier.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::time::Duration;

/// How often the job runs. Matches the finest tier so a 5-minute bucket is picked up
/// on the next pass after it closes.
const TICK: Duration = Duration::from_secs(300);

/// Most time one pass will fill per tier. Without a cap the very first pass after a
/// deploy would scan every raw row at once; with it the backlog drains over a few
/// minutes instead of stalling a connection on a tens-of-millions-of-rows scan.
const MAX_SPAN_5M: &str = "6 hours";
const MAX_SPAN_1H: &str = "7 days";

/// Where a tier starts when it has never been filled — i.e. the first run after this
/// feature ships. Raw is only kept a couple of days anyway, so there is nothing older
/// to recover.
const COLD_START: &str = "2 days";

pub fn spawn(data: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            run_once(&data).await;
        }
    });
}

/// One full pass: fill 5m from raw, then 1h from 5m. Best-effort — a failure is logged
/// and the progress marker is left untouched, so the next pass retries the same window
/// rather than skipping it.
pub async fn run_once(data: &PgPool) {
    if let Err(e) = fill_5m(data).await {
        tracing::warn!(error = %e, "kube rollup: 5m fill failed");
        return;
    }
    if let Err(e) = fill_1h(data).await {
        tracing::warn!(error = %e, "kube rollup: 1h fill failed");
    }
}

/// Last bucket boundary a tier has been filled up to (exclusive).
async fn progress(data: &PgPool, tier: &str, cold_start: &str) -> DateTime<Utc> {
    let row: Option<(DateTime<Utc>,)> =
        sqlx::query_as("SELECT filled_to FROM kube_rollup_progress WHERE tier = $1")
            .bind(tier)
            .fetch_optional(data)
            .await
            .ok()
            .flatten();
    match row {
        Some((t,)) => t,
        None => Utc::now() - chrono::Duration::seconds(parse_days_secs(cold_start)),
    }
}

/// Only used for the cold-start constant above, which is a fixed literal.
fn parse_days_secs(s: &str) -> i64 {
    match s {
        "2 days" => 2 * 86400,
        _ => 86400,
    }
}

async fn set_progress(data: &PgPool, tier: &str, to: DateTime<Utc>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO kube_rollup_progress (tier, filled_to) VALUES ($1, $2) \
         ON CONFLICT (tier) DO UPDATE SET filled_to = EXCLUDED.filled_to",
    )
    .bind(tier)
    .bind(to)
    .execute(data)
    .await
    .map(|_| ())
}

/// raw → 5m. Stops at the last CLOSED bucket: rolling up the bucket currently being
/// written would store a partial average and `ON CONFLICT DO NOTHING` would then keep
/// that wrong value forever.
async fn fill_5m(data: &PgPool) -> Result<(), sqlx::Error> {
    let from = progress(data, "5m", COLD_START).await;
    let (to,): (DateTime<Utc>,) = sqlx::query_as(&format!(
        "SELECT least(time_bucket(INTERVAL '5 minutes', now()), $1::timestamptz + INTERVAL '{MAX_SPAN_5M}')"
    ))
    .bind(from)
    .fetch_one(data)
    .await?;
    if to <= from {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO kube_rollup_5m \
           (bucket, system_id, namespace, workload, workload_kind, node, \
            cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts) \
         SELECT b, system_id, namespace, workload, workload_kind, node, \
                avg(cpu), max(cpu), avg(mem), max(mem), avg(n), max(restarts) \
         FROM ( \
           SELECT time_bucket(INTERVAL '5 minutes', time) AS b, time, system_id, \
                  namespace, workload, workload_kind, node, \
                  sum(cpu_millicores)::float8 AS cpu, sum(mem_bytes)::float8 AS mem, \
                  count(*)::float8 AS n, max(restarts) AS restarts \
           FROM kube_container_stats \
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
    set_progress(data, "5m", to).await
}

/// 5m → 1h. Never runs past what the 5m tier has actually filled, or an hour whose
/// later minutes have not been rolled up yet would be stored short and then frozen by
/// `ON CONFLICT DO NOTHING`.
async fn fill_1h(data: &PgPool) -> Result<(), sqlx::Error> {
    let from = progress(data, "1h", COLD_START).await;
    let filled_5m = progress(data, "5m", COLD_START).await;
    let (to,): (DateTime<Utc>,) = sqlx::query_as(&format!(
        "SELECT least(time_bucket(INTERVAL '1 hour', $1::timestamptz), \
                      $2::timestamptz + INTERVAL '{MAX_SPAN_1H}')"
    ))
    .bind(filled_5m)
    .bind(from)
    .fetch_one(data)
    .await?;
    if to <= from {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO kube_rollup_1h \
           (bucket, system_id, namespace, workload, workload_kind, node, \
            cpu_avg, cpu_max, mem_avg, mem_max, containers, restarts) \
         SELECT time_bucket(INTERVAL '1 hour', bucket), system_id, namespace, workload, \
                workload_kind, node, \
                avg(cpu_avg), max(cpu_max), avg(mem_avg), max(mem_max), \
                avg(containers), max(restarts) \
         FROM kube_rollup_5m \
         WHERE bucket >= $1 AND bucket < $2 \
         GROUP BY 1, 2, 3, 4, 5, 6 \
         ON CONFLICT DO NOTHING",
    )
    .bind(from)
    .bind(to)
    .execute(data)
    .await?;
    set_progress(data, "1h", to).await
}
