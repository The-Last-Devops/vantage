//! Data lifecycle management: TimescaleDB continuous aggregates (downsampling)
//! + retention tiers, plus DB-size / retention introspection for the admin UI.
//!
//! Tiers (defaults): raw samples kept short, 1-minute rollup mid, 1-hour rollup long.
//! Continuous aggregates can't be created inside a migration transaction, so we
//! set them up at startup over the autocommit pool (idempotent, best-effort).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;

/// Best-effort downsampling setup. Errors (e.g. policy already exists) are logged
/// and ignored so startup never fails on re-run.
// Aggregation expressions reused at every rollup level — column names match the
// raw table, so the SAME expressions roll a finer tier up into a coarser one
// (avg of avgs over equal-width buckets; max of cumulative counters stays a max).
const SYS_AGG: &str = "avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, \
     avg(mem_total) AS mem_total, avg(swap_used) AS swap_used, avg(swap_total) AS swap_total, \
     avg(disk_used) AS disk_used, avg(disk_total) AS disk_total, \
     max(net_rx) AS net_rx, max(net_tx) AS net_tx, max(disk_read) AS disk_read, max(disk_write) AS disk_write, \
     avg(load1) AS load1, avg(load5) AS load5, avg(load15) AS load15, \
     avg(cpu_user) AS cpu_user, avg(cpu_system) AS cpu_system, avg(cpu_iowait) AS cpu_iowait, \
     avg(cpu_steal) AS cpu_steal, avg(disk_util) AS disk_util, \
     avg(mem_available) AS mem_available, avg(mem_buffers) AS mem_buffers, \
     avg(mem_cached) AS mem_cached, avg(mem_free) AS mem_free";
const CTR_AGG: &str = "avg(cpu_percent) AS cpu_percent, avg(mem_used) AS mem_used, \
     max(net_rx) AS net_rx, max(net_tx) AS net_tx";

/// One rollup tier: (suffix, bucket, source-table, source-time-column).
const SYS_TIERS: &[(&str, &str, &str, &str)] = &[
    ("1m", "1 minute", "system_metrics", "time"),
    ("5m", "5 minutes", "system_metrics_1m", "bucket"),
    ("15m", "15 minutes", "system_metrics_5m", "bucket"),
    ("1h", "1 hour", "system_metrics_15m", "bucket"),
];
const CTR_TIERS: &[(&str, &str, &str, &str)] = &[
    ("1m", "1 minute", "container_metrics", "time"),
    ("5m", "5 minutes", "container_metrics_1m", "bucket"),
    ("15m", "15 minutes", "container_metrics_5m", "bucket"),
    ("1h", "1 hour", "container_metrics_15m", "bucket"),
];
/// Refresh start_offset per tier — must stay within the SOURCE tier's retention
/// (raw 8h, 1m 2d, 5m 10d, 15m 45d) so a refresh never blanks materialized rows.
const REFRESH_OFFSET: &[(&str, &str)] = &[
    ("1m", "6 hours"),
    ("5m", "1 day"),
    ("15m", "3 days"),
    ("1h", "14 days"),
];
/// drop_after per tier (raw is hours, rollups are days). Heartbeats handled separately.
const RETENTION: &[(&str, &str)] = &[
    ("", "8 hours"),
    ("1m", "2 days"),
    ("5m", "10 days"),
    ("15m", "45 days"),
    ("1h", "365 days"),
];
const COMPRESS_AFTER: &[(&str, &str)] = &[
    ("1m", "1 day"),
    ("5m", "2 days"),
    ("15m", "7 days"),
    ("1h", "14 days"),
];

pub async fn setup(data: &PgPool) {
    // Pre-ladder schema had only _1m/_1h rollups (8 columns, _1h sourced from raw).
    // If the _5m tier is absent we're upgrading (or fresh): drop the old rollup chain
    // so it's recreated with the full column set + hierarchical sources, and reset
    // retention so the new defaults apply. After this _5m exists → block is skipped,
    // leaving any admin retention edits intact.
    let has_5m = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT to_regclass('public.system_metrics_5m')::text",
    )
    .fetch_one(data)
    .await
    .ok()
    .and_then(|(v,)| v)
    .is_some();
    // Also recreate the ladder when the rollups predate a new raw column (e.g. the
    // memory breakdown) — continuous aggregates can't be ALTERed, so a column added to
    // SYS_AGG only lands by dropping + rebuilding. Detect via the 1m view's columns.
    let has_mem_cols = sqlx::query_as::<_, (Option<i32>,)>(
        "SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'system_metrics_1m' AND column_name = 'mem_available'",
    )
    .fetch_optional(data)
    .await
    .ok()
    .flatten()
    .is_some();
    let migrated = has_5m && has_mem_cols;
    if !migrated {
        let mut reset = vec![
            "DROP MATERIALIZED VIEW IF EXISTS system_metrics_1h CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS system_metrics_15m CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS system_metrics_5m CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS system_metrics_1m CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS container_metrics_1h CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS container_metrics_15m CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS container_metrics_5m CASCADE".to_string(),
            "DROP MATERIALIZED VIEW IF EXISTS container_metrics_1m CASCADE".to_string(),
        ];
        for t in RETENTION_TABLES {
            reset.push(format!(
                "SELECT remove_retention_policy('{t}', if_exists => true)"
            ));
        }
        for s in &reset {
            let _ = sqlx::query(s).execute(data).await;
        }
    }

    let mut stmts: Vec<String> = vec![
        // Raw is the short hot tier (8h) → 1h chunks so retention drops at that grain.
        "SELECT set_chunk_time_interval('system_metrics', INTERVAL '1 hour')".into(),
        "SELECT set_chunk_time_interval('container_metrics', INTERVAL '1 hour')".into(),
        "SELECT set_chunk_time_interval('heartbeats', INTERVAL '1 day')".into(),
    ];

    // Hierarchical rollup chains: raw → 1m → 5m → 15m → 1h (system + container).
    for (chain, agg, group_extra) in [(SYS_TIERS, SYS_AGG, ""), (CTR_TIERS, CTR_AGG, "name, ")] {
        let table_base = if group_extra.is_empty() {
            "system_metrics"
        } else {
            "container_metrics"
        };
        for (suffix, bucket, src, srccol) in chain {
            stmts.push(format!(
                "CREATE MATERIALIZED VIEW IF NOT EXISTS {table_base}_{suffix} \
                 WITH (timescaledb.continuous) AS \
                 SELECT system_id, {group_extra}time_bucket('{bucket}', {srccol}) AS bucket, {agg} \
                 FROM {src} GROUP BY system_id, {group_extra}time_bucket('{bucket}', {srccol}) WITH NO DATA"
            ));
        }
        for (suffix, bucket, _, _) in chain {
            let off = REFRESH_OFFSET.iter().find(|(s, _)| s == suffix).unwrap().1;
            stmts.push(format!(
                "SELECT add_continuous_aggregate_policy('{table_base}_{suffix}', \
                    start_offset => INTERVAL '{off}', end_offset => INTERVAL '{bucket}', \
                    schedule_interval => INTERVAL '{bucket}')"
            ));
        }
        // retention + compression per tier
        for (suffix, keep) in RETENTION {
            let tbl = if suffix.is_empty() {
                table_base.to_string()
            } else {
                format!("{table_base}_{suffix}")
            };
            stmts.push(format!(
                "SELECT add_retention_policy('{tbl}', INTERVAL '{keep}')"
            ));
        }
        for (suffix, after) in COMPRESS_AFTER {
            stmts.push(format!(
                "ALTER MATERIALIZED VIEW {table_base}_{suffix} SET (timescaledb.compress = true)"
            ));
            stmts.push(format!(
                "SELECT add_compression_policy('{table_base}_{suffix}', INTERVAL '{after}')"
            ));
        }
    }

    // Heartbeats: kept a year so uptime history + incidents span long ranges.
    stmts.push("SELECT add_retention_policy('heartbeats', INTERVAL '365 days')".into());
    stmts.push(
        "ALTER TABLE heartbeats SET (timescaledb.compress, \
            timescaledb.compress_segmentby = 'monitor_id', timescaledb.compress_orderby = 'time DESC')"
            .into(),
    );
    stmts.push("SELECT add_compression_policy('heartbeats', INTERVAL '7 days')".into());

    // Kubernetes cluster-state series: keep raw a year with daily chunks +
    // compression, no rollup ladder for now. kube_container_stats is per-container
    // (higher volume) but compresses hard (labels/metadata repeat per snapshot).
    for tbl in [
        "kube_namespace_stats",
        "kube_deployment_stats",
        "kube_container_stats",
    ] {
        stmts.push(format!(
            "SELECT set_chunk_time_interval('{tbl}', INTERVAL '1 day')"
        ));
        stmts.push(format!(
            "SELECT add_retention_policy('{tbl}', INTERVAL '365 days')"
        ));
        stmts.push(format!(
            "ALTER TABLE {tbl} SET (timescaledb.compress, \
                timescaledb.compress_segmentby = 'system_id', timescaledb.compress_orderby = 'time DESC')"
        ));
        stmts.push(format!(
            "SELECT add_compression_policy('{tbl}', INTERVAL '7 days')"
        ));
    }

    for s in &stmts {
        if let Err(e) = sqlx::query(s).execute(data).await {
            tracing::debug!(error = %e, "downsampling setup (ignored)");
        }
    }
    tracing::info!("downsampling ladder (1m/5m/15m/1h) + retention + compression configured");
}

#[derive(Serialize)]
pub struct TableStat {
    pub name: String,
    pub size: String,
    pub rows: i64,
    /// What the table is for (config DB only; None for the data-DB tiers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Configured cleanup window in days for the time-growing log tables; None when
    /// the table isn't auto-pruned. Editable via POST /api/admin/config-retention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i32>,
}

#[derive(Serialize)]
pub struct RetentionTier {
    pub table: String,
    pub label: String,
    /// "hours" for the raw realtime tier, "days" for the downsampled tiers.
    pub unit: String,
    pub value: Option<i64>,
}

/// The raw realtime tiers (system + container) are managed in hours; the
/// downsampled rollups + heartbeats in days.
fn unit_for(table: &str) -> &'static str {
    if table == "system_metrics" || table == "container_metrics" {
        "hours"
    } else {
        "days"
    }
}

/// A plain relational DB's stats (used for the config DB — no hypertables/retention).
#[derive(Serialize)]
pub struct DbStats {
    pub db_size: String,
    pub tables: Vec<TableStat>,
}

/// Hard-cap status for the Data DB. `used_bytes` is the live `pg_database_size`.
#[derive(Serialize)]
pub struct CapStatus {
    pub limit_bytes: i64,
    pub used_bytes: i64,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct DataDbStats {
    pub db_size: String,
    pub tables: Vec<TableStat>,
    pub retention: Vec<RetentionTier>,
    pub cap: CapStatus,
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
        note: None,
        retention_days: None,
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

pub async fn data_stats(config: &PgPool, data: &PgPool) -> DataDbStats {
    let db_size = sqlx::query_as::<_, (String,)>(
        "SELECT pg_size_pretty(pg_database_size(current_database()))",
    )
    .fetch_one(data)
    .await
    .map(|(s,)| s)
    .unwrap_or_else(|_| "—".into());

    // (table, label) for each tier — used for both the size table and retention.
    let tiers = [
        ("system_metrics", "Raw (realtime)"),
        ("system_metrics_1m", "1-minute rollup"),
        ("system_metrics_5m", "5-minute rollup"),
        ("system_metrics_15m", "15-minute rollup"),
        ("system_metrics_1h", "1-hour rollup"),
        ("container_metrics", "Container (raw)"),
        ("container_metrics_1m", "Container 1-minute"),
        ("container_metrics_5m", "Container 5-minute"),
        ("container_metrics_15m", "Container 15-minute"),
        ("container_metrics_1h", "Container 1-hour"),
        ("heartbeats", "Heartbeats"),
        ("kube_namespace_stats", "K8s namespaces"),
        ("kube_deployment_stats", "K8s deployments"),
        ("kube_container_stats", "K8s containers"),
    ];
    let mut tables = Vec::with_capacity(tiers.len());
    for (table, label) in tiers {
        tables.push(hypertable_stat(data, table, label).await);
    }
    let mut retention = Vec::with_capacity(tiers.len());
    for (table, label) in tiers {
        retention.push(RetentionTier {
            table: table.into(),
            label: label.into(),
            unit: unit_for(table).into(),
            value: retention_value(data, table).await,
        });
    }

    DataDbStats {
        db_size,
        tables,
        retention,
        cap: cap_status(config, data).await,
    }
}

/// Live size of the given database in bytes.
async fn db_size_bytes(pool: &PgPool) -> i64 {
    sqlx::query_as::<_, (i64,)>("SELECT pg_database_size(current_database())")
        .fetch_one(pool)
        .await
        .map(|(n,)| n)
        .unwrap_or(0)
}

/// Per-table stats for a plain relational DB (the config DB). The config DB is small,
/// so we run an exact `COUNT(*)` per table — the planner's `reltuples` estimate reads 0
/// for any table not yet ANALYZEd (e.g. users/workspaces with low write volume), which
/// looks like the table is empty when it isn't.
pub async fn config_stats(config: &PgPool) -> DbStats {
    let db_size = sqlx::query_as::<_, (String,)>(
        "SELECT pg_size_pretty(pg_database_size(current_database()))",
    )
    .fetch_one(config)
    .await
    .map(|(s,)| s)
    .unwrap_or_else(|_| "—".into());
    // Table name + exact on-disk size from the catalog, biggest first.
    let metas: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname, pg_size_pretty(pg_total_relation_size(c.oid)) \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind = 'r' \
         ORDER BY pg_total_relation_size(c.oid) DESC",
    )
    .fetch_all(config)
    .await
    .unwrap_or_default();
    // Configured retention (days) for the auto-pruned log tables, by table name.
    let mut retention: std::collections::HashMap<&str, i32> = std::collections::HashMap::new();
    for (table, _, key) in CONFIG_LOGS {
        if let Some(d) = crate::settings::get_opt::<i32>(config, key).await {
            retention.insert(table, d);
        }
    }
    let mut tables = Vec::with_capacity(metas.len());
    for (name, size) in metas {
        // Names come from pg_catalog (our own schema); double-quote defensively.
        let rows = sqlx::query_as::<_, (i64,)>(&format!(
            "SELECT count(*) FROM \"{}\"",
            name.replace('"', "\"\"")
        ))
        .fetch_one(config)
        .await
        .map(|(n,)| n)
        .unwrap_or(0);
        let retention_days = retention.get(name.as_str()).copied();
        let note = config_table_note(&name).map(String::from);
        tables.push(TableStat {
            name,
            size,
            rows,
            note,
            retention_days,
        });
    }
    DbStats { db_size, tables }
}

/// One-line purpose for each known config-DB table (shown in the admin UI).
fn config_table_note(name: &str) -> Option<&'static str> {
    Some(match name {
        "exec_transcript" => "Recorded terminal output of each SSH/console session (audit trail)",
        "exec_sessions" => "One row per shell/console session — who, which host, when",
        "alert_events" => "History of alert state changes (fired / resolved)",
        "audit_log" => "Log of mutating user actions — who did what",
        "sessions" => "Active browser login sessions (httpOnly cookie tokens)",
        "monitor_debug" => "Latest probe debug detail per monitor (overwritten, not history)",
        "alert_state" => "Current firing state per alert rule",
        "systems" => "Monitored hosts/servers and their agent + shell config",
        "monitors" => "Service checks (HTTP, TCP, ping, DNS, …)",
        "workspaces" => "Tenancy boundaries that RBAC is scoped to",
        "users" => "User accounts (argon2 password hashes, admin flag)",
        "memberships" => "User × workspace role (owner/editor/viewer, can_exec)",
        "channels" => "Notification channels (Slack, webhook, email, …)",
        "alerts" => "Alert rules — conditions and what they notify",
        "alert_channels" => "Which channels each alert rule notifies",
        "status_pages" => "Public status-page definitions",
        "ssh_keys" => "User SSH keys for host consoles (encrypted at rest)",
        "webauthn_credentials" => "Registered passkeys (WebAuthn credentials)",
        "api_keys" => "Agent enrollment tokens (one per server)",
        "api_pats" => "Personal access tokens for the API / MCP (Bearer auth)",
        "settings" => "Hub-wide settings (key→value: backup, retention, cap, S3)",
        "_sqlx_migrations" => "Applied database-migration history",
        _ => return None,
    })
}

/// (table, time column, setting key) for the time-growing config-DB logs that the
/// background job prunes. Table/column names are a fixed allowlist — safe to inline.
const CONFIG_LOGS: &[(&str, &str, &str)] = &[
    ("exec_transcript", "at", "exec_transcript_retention_days"),
    ("alert_events", "at", "alert_events_retention_days"),
    (
        "exec_sessions",
        "started_at",
        "exec_sessions_retention_days",
    ),
    ("sessions", "created_at", "sessions_retention_days"),
];

/// Update one config-log table's retention window (days, 1..=3650).
pub async fn set_config_retention(config: &PgPool, table: &str, days: i64) -> Result<(), String> {
    let Some((_, _, col)) = CONFIG_LOGS.iter().find(|(t, _, _)| *t == table) else {
        return Err("invalid table".into());
    };
    if !(1..=3650).contains(&days) {
        return Err("value out of range".into());
    }
    crate::settings::set(config, col, &(days as i32))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Spawn the config-DB log pruner: deletes rows older than each table's configured
/// retention window, hourly. Best-effort; errors are logged and ignored.
pub fn spawn_config_prune(config: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            prune_config_logs(&config).await;
        }
    });
}

/// One prune pass over the config-DB log tables.
pub async fn prune_config_logs(config: &PgPool) {
    for (table, tcol, key) in CONFIG_LOGS {
        let days = crate::settings::get_opt::<i32>(config, key).await;
        if let Some(d) = days {
            if d > 0 {
                if let Err(e) = sqlx::query(&format!(
                    "DELETE FROM {table} WHERE {tcol} < now() - interval '{d} days'"
                ))
                .execute(config)
                .await
                {
                    tracing::debug!(error = %e, %table, "config-log prune (ignored)");
                }
            }
        }
    }
}

/// Current cap config (from the config DB) + live Data-DB usage.
pub async fn cap_status(config: &PgPool, data: &PgPool) -> CapStatus {
    let limit_bytes =
        crate::settings::get(config, "data_cap_limit_bytes", 10_737_418_240_i64).await;
    let enabled = crate::settings::get(config, "data_cap_enabled", false).await;
    CapStatus {
        limit_bytes,
        used_bytes: db_size_bytes(data).await,
        enabled,
    }
}

/// Cap bounds: 256 MiB .. 1 TiB.
const CAP_MIN: i64 = 256 * 1024 * 1024;
const CAP_MAX: i64 = 1024 * 1024 * 1024 * 1024;

/// Update the Data-DB cap (config DB). `limit_bytes` must be within [256 MiB, 1 TiB].
pub async fn set_data_cap(config: &PgPool, limit_bytes: i64, enabled: bool) -> Result<(), String> {
    if !(CAP_MIN..=CAP_MAX).contains(&limit_bytes) {
        return Err("limit out of range".into());
    }
    crate::settings::set(config, "data_cap_limit_bytes", &limit_bytes)
        .await
        .map_err(|e| e.to_string())?;
    crate::settings::set(config, "data_cap_enabled", &enabled)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn human_bytes(n: i64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

/// Spawn the background cap enforcer: every 5 minutes, if the cap is enabled and the
/// Data DB exceeds it, drop the oldest chunks until back under (or no progress).
pub fn spawn_enforce(config: PgPool, data: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            enforce_cap(&config, &data).await;
        }
    });
}

/// One enforcement pass. Reads the cap from the config DB; if enabled and the Data DB
/// is over its limit, drops the globally-oldest time chunks (every hypertable, oldest
/// history first) until back under the limit, a chunk can't be found, no space is
/// freed, or a safety floor (~50 drops) is hit. Never touches the config DB; only ever
/// calls `drop_chunks` on the Data DB's hypertables. No-op when disabled.
pub async fn enforce_cap(config: &PgPool, data: &PgPool) {
    let limit = crate::settings::get(config, "data_cap_limit_bytes", 10_737_418_240_i64).await;
    let enabled = crate::settings::get(config, "data_cap_enabled", false).await;
    if !enabled {
        return;
    }
    let start = db_size_bytes(data).await;
    if start <= limit {
        return;
    }
    let mut used = start;
    for _ in 0..50 {
        if used <= limit {
            break;
        }
        // Oldest chunk across ALL hypertables (oldest history first, any tier).
        let oldest: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT hypertable_name, range_end FROM timescaledb_information.chunks \
             ORDER BY range_end ASC LIMIT 1",
        )
        .fetch_optional(data)
        .await
        .ok()
        .flatten();
        let Some((ht, range_end)) = oldest else {
            break; // nothing left to drop
        };
        if let Err(e) = sqlx::query("SELECT drop_chunks(format('%I', $1), older_than => $2)")
            .bind(&ht)
            .bind(range_end)
            .execute(data)
            .await
        {
            tracing::warn!(error = %e, hypertable = %ht, "data cap: drop_chunks failed");
            break;
        }
        let after = db_size_bytes(data).await;
        if after >= used {
            break; // no progress (chunk freed nothing reclaimable yet) — avoid a tight loop
        }
        used = after;
    }
    if used < start {
        let msg = format!(
            "data cap eviction: freed {}, now {} / limit {}",
            human_bytes(start - used),
            human_bytes(used),
            human_bytes(limit)
        );
        tracing::warn!("{msg}");
        let _ = sqlx::query(
            "INSERT INTO audit_log (user_email, method, path, status, object_name) \
             VALUES ('system', 'EVICT', '/api/admin/data-cap', 200, $1)",
        )
        .bind(&msg)
        .execute(config)
        .await;
    }
}

/// Allowlist of tables whose retention may be changed from the UI.
const RETENTION_TABLES: &[&str] = &[
    "system_metrics",
    "system_metrics_1m",
    "system_metrics_5m",
    "system_metrics_15m",
    "system_metrics_1h",
    "container_metrics",
    "container_metrics_1m",
    "container_metrics_5m",
    "container_metrics_15m",
    "container_metrics_1h",
    "heartbeats",
    "kube_namespace_stats",
    "kube_deployment_stats",
    "kube_container_stats",
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
