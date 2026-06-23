//! Data lifecycle management: TimescaleDB continuous aggregates (downsampling)
//! + retention tiers, plus DB-size / retention introspection for the admin UI.
//!
//! Tiers (defaults): raw samples kept short, 1-minute rollup mid, 1-hour rollup long.
//! Continuous aggregates can't be created inside a migration transaction, so we
//! set them up at startup over the autocommit pool (idempotent, best-effort).

use serde::Serialize;
use sqlx::PgPool;

/// Best-effort downsampling setup. Errors (e.g. policy already exists) are logged
/// and ignored so startup never fails on re-run.
pub async fn setup(data: &PgPool) {
    let stmts = [
        // Raw system_metrics is the short, hour-granularity hot tier: small chunks so
        // an hours-based retention policy can actually drop data (default kept 24h).
        "SELECT set_chunk_time_interval('system_metrics', INTERVAL '1 hour')",
        // Container raw + heartbeats keep day chunks (longer retention).
        "SELECT set_chunk_time_interval('container_metrics', INTERVAL '1 day')",
        "SELECT set_chunk_time_interval('heartbeats', INTERVAL '1 day')",
        // 1-minute rollup from raw system_metrics.
        "CREATE MATERIALIZED VIEW IF NOT EXISTS system_metrics_1m \
         WITH (timescaledb.continuous) AS \
         SELECT system_id, time_bucket('1 minute', time) AS bucket, \
                avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, avg(mem_total) AS mem_total, \
                avg(disk_used) AS disk_used, avg(disk_total) AS disk_total, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx, avg(load1) AS load1 \
         FROM system_metrics GROUP BY system_id, bucket WITH NO DATA",
        // 1-hour rollup from raw system_metrics.
        "CREATE MATERIALIZED VIEW IF NOT EXISTS system_metrics_1h \
         WITH (timescaledb.continuous) AS \
         SELECT system_id, time_bucket('1 hour', time) AS bucket, \
                avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, avg(mem_total) AS mem_total, \
                avg(disk_used) AS disk_used, avg(disk_total) AS disk_total, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx, avg(load1) AS load1 \
         FROM system_metrics GROUP BY system_id, bucket WITH NO DATA",
        // Refresh policies.
        "SELECT add_continuous_aggregate_policy('system_metrics_1m', start_offset => INTERVAL '6 hours', \
            end_offset => INTERVAL '1 minute', schedule_interval => INTERVAL '1 minute')",
        "SELECT add_continuous_aggregate_policy('system_metrics_1h', start_offset => INTERVAL '3 days', \
            end_offset => INTERVAL '1 hour', schedule_interval => INTERVAL '1 hour')",
        // Container rollups (parity with system_metrics): 1-minute from raw, then a
        // 1-hour rollup built hierarchically from the 1-minute one so it survives a
        // short raw retention. Grouped per (system_id, name).
        "CREATE MATERIALIZED VIEW IF NOT EXISTS container_metrics_1m \
         WITH (timescaledb.continuous) AS \
         SELECT system_id, name, time_bucket('1 minute', time) AS bucket, \
                avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx \
         FROM container_metrics GROUP BY system_id, name, bucket WITH NO DATA",
        "CREATE MATERIALIZED VIEW IF NOT EXISTS container_metrics_1h \
         WITH (timescaledb.continuous) AS \
         SELECT system_id, name, time_bucket('1 hour', bucket) AS bucket, \
                avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, \
                max(net_rx) AS net_rx, max(net_tx) AS net_tx \
         FROM container_metrics_1m GROUP BY system_id, name, time_bucket('1 hour', bucket) WITH NO DATA",
        "SELECT add_continuous_aggregate_policy('container_metrics_1m', start_offset => INTERVAL '6 hours', \
            end_offset => INTERVAL '1 minute', schedule_interval => INTERVAL '1 minute')",
        "SELECT add_continuous_aggregate_policy('container_metrics_1h', start_offset => INTERVAL '3 days', \
            end_offset => INTERVAL '1 hour', schedule_interval => INTERVAL '1 hour')",
        // Retention tiers (raw system_metrics defaults to 24h, editable in hours).
        "SELECT add_retention_policy('system_metrics', INTERVAL '24 hours')",
        "SELECT add_retention_policy('system_metrics_1m', INTERVAL '7 days')",
        "SELECT add_retention_policy('system_metrics_1h', INTERVAL '30 days')",
        "SELECT add_retention_policy('container_metrics', INTERVAL '7 days')",
        "SELECT add_retention_policy('container_metrics_1m', INTERVAL '7 days')",
        "SELECT add_retention_policy('container_metrics_1h', INTERVAL '30 days')",
        "SELECT add_retention_policy('heartbeats', INTERVAL '30 days')",
        // Compression (append-only data → ~pure win). Raw system_metrics is the
        // short hot tier (1-day retention) so it stays uncompressed; compress the
        // longer-lived rollups + container/heartbeat history.
        "ALTER TABLE container_metrics SET (timescaledb.compress, \
            timescaledb.compress_segmentby = 'system_id', timescaledb.compress_orderby = 'time DESC')",
        "SELECT add_compression_policy('container_metrics', INTERVAL '2 days')",
        "ALTER TABLE heartbeats SET (timescaledb.compress, \
            timescaledb.compress_segmentby = 'monitor_id', timescaledb.compress_orderby = 'time DESC')",
        "SELECT add_compression_policy('heartbeats', INTERVAL '2 days')",
        "ALTER MATERIALIZED VIEW system_metrics_1m SET (timescaledb.compress = true)",
        "SELECT add_compression_policy('system_metrics_1m', INTERVAL '1 day')",
        "ALTER MATERIALIZED VIEW system_metrics_1h SET (timescaledb.compress = true)",
        "SELECT add_compression_policy('system_metrics_1h', INTERVAL '7 days')",
        "ALTER MATERIALIZED VIEW container_metrics_1m SET (timescaledb.compress = true)",
        "SELECT add_compression_policy('container_metrics_1m', INTERVAL '1 day')",
        "ALTER MATERIALIZED VIEW container_metrics_1h SET (timescaledb.compress = true)",
        "SELECT add_compression_policy('container_metrics_1h', INTERVAL '7 days')",
    ];
    for s in stmts {
        if let Err(e) = sqlx::query(s).execute(data).await {
            tracing::debug!(error = %e, "downsampling setup (ignored)");
        }
    }
    tracing::info!("downsampling + retention + compression configured");
}

#[derive(Serialize)]
pub struct TableStat {
    pub name: String,
    pub size: String,
    pub rows: i64,
}

#[derive(Serialize)]
pub struct RetentionTier {
    pub table: String,
    pub label: String,
    /// "hours" for the raw realtime tier, "days" for the downsampled tiers.
    pub unit: String,
    pub value: Option<i64>,
}

/// The raw realtime tier is managed in hours; everything else in days.
fn unit_for(table: &str) -> &'static str {
    if table == "system_metrics" {
        "hours"
    } else {
        "days"
    }
}

#[derive(Serialize)]
pub struct DataStats {
    pub db_size: String,
    pub tables: Vec<TableStat>,
    pub retention: Vec<RetentionTier>,
}

async fn hypertable_stat(data: &PgPool, name: &str, label: &str) -> TableStat {
    let size: Option<(String,)> = sqlx::query_as("SELECT pg_size_pretty(hypertable_size($1))")
        .bind(name)
        .fetch_optional(data)
        .await
        .ok()
        .flatten();
    let rows: Option<(i64,)> = sqlx::query_as(&format!("SELECT count(*) FROM {name}"))
        .fetch_optional(data)
        .await
        .ok()
        .flatten();
    TableStat {
        name: label.to_string(),
        size: size.map(|(s,)| s).unwrap_or_else(|| "—".into()),
        rows: rows.map(|(r,)| r).unwrap_or(0),
    }
}

/// Reads a retention policy's `drop_after` interval for a hypertable, expressed
/// in the tier's unit (hours for the raw tier, days otherwise).
async fn retention_value(data: &PgPool, table: &str) -> Option<i64> {
    let divisor = if unit_for(table) == "hours" {
        3600
    } else {
        86400
    };
    let row: Option<(Option<i64>,)> = sqlx::query_as(&format!(
        "SELECT (EXTRACT(EPOCH FROM (config->>'drop_after')::interval) / {divisor})::bigint \
         FROM timescaledb_information.jobs \
         WHERE proc_name = 'policy_retention' AND hypertable_name = $1"
    ))
    .bind(table)
    .fetch_optional(data)
    .await
    .ok()
    .flatten();
    row.and_then(|(d,)| d)
}

pub async fn stats(data: &PgPool) -> DataStats {
    let db_size = sqlx::query_as::<_, (String,)>(
        "SELECT pg_size_pretty(pg_database_size(current_database()))",
    )
    .fetch_one(data)
    .await
    .map(|(s,)| s)
    .unwrap_or_else(|_| "—".into());

    let tables = vec![
        hypertable_stat(data, "system_metrics", "Raw metrics").await,
        hypertable_stat(data, "system_metrics_1m", "1-minute rollup").await,
        hypertable_stat(data, "system_metrics_1h", "1-hour rollup").await,
        hypertable_stat(data, "container_metrics", "Container metrics").await,
        hypertable_stat(data, "container_metrics_1m", "Container 1-minute rollup").await,
        hypertable_stat(data, "container_metrics_1h", "Container 1-hour rollup").await,
        hypertable_stat(data, "heartbeats", "Heartbeats").await,
    ];

    // (table, label) for each tier; unit + value are filled in below.
    let tiers = [
        ("system_metrics", "Raw (realtime)"),
        ("system_metrics_1m", "1-minute rollup"),
        ("system_metrics_1h", "1-hour rollup"),
        ("container_metrics", "Container (raw)"),
        ("container_metrics_1m", "Container 1-minute rollup"),
        ("container_metrics_1h", "Container 1-hour rollup"),
        ("heartbeats", "Heartbeats"),
    ];
    let mut retention = Vec::with_capacity(tiers.len());
    for (table, label) in tiers {
        retention.push(RetentionTier {
            table: table.into(),
            label: label.into(),
            unit: unit_for(table).into(),
            value: retention_value(data, table).await,
        });
    }

    DataStats {
        db_size,
        tables,
        retention,
    }
}

/// Allowlist of tables whose retention may be changed from the UI.
const RETENTION_TABLES: &[&str] = &[
    "system_metrics",
    "system_metrics_1m",
    "system_metrics_1h",
    "container_metrics",
    "container_metrics_1m",
    "container_metrics_1h",
    "heartbeats",
];

/// `value` is interpreted in the tier's unit (hours for the raw tier, days else).
pub async fn set_retention(data: &PgPool, table: &str, value: i64) -> Result<(), String> {
    if !RETENTION_TABLES.contains(&table) {
        return Err("invalid table".into());
    }
    let unit = unit_for(table);
    let max = if unit == "hours" { 8760 } else { 3650 }; // 1y in hours / 10y in days
    if !(1..=max).contains(&value) {
        return Err("value out of range".into());
    }
    let _ = sqlx::query(&format!(
        "SELECT remove_retention_policy('{table}', if_exists => true)"
    ))
    .execute(data)
    .await;
    sqlx::query(&format!(
        "SELECT add_retention_policy('{table}', INTERVAL '{value} {unit}')"
    ))
    .execute(data)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
