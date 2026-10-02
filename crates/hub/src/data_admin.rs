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

// Retention + compression for every tier, applied at startup (idempotent, best-effort).
//
// There is no continuous aggregate anywhere in here any more: the rollup tiers are plain
// tables filled by `rollup.rs`. A CAgg cannot be `ALTER`ed, so adding one metric column
// forced a drop and rebuild of the whole chain, and the long tier could then only refill
// from the tier below — the reason `system_metrics_1h` held 43 days under a 365-day
// policy. See `migrations/data/0004`.

/// Every tier: (table, keep, compress_after, time column, compress segment-by).
///
/// `compress_after` MUST be comfortably shorter than `keep`, or chunks reach the drop
/// age at the same moment they become eligible and nothing is ever compressed. That
/// exact mistake shipped in 3.2.0 for the k8s detail tier. `None` means the tier is too
/// short-lived for compression to pay for itself.
const TIERS: &[(&str, &str, Option<&str>, &str, &str)] = &[
    ("system_metrics_5s", "8 hours", None, "time", "system_id"),
    (
        "system_metrics_1m",
        "2 days",
        Some("1 day"),
        "bucket",
        "system_id",
    ),
    (
        "system_metrics_1h",
        "365 days",
        Some("14 days"),
        "bucket",
        "system_id",
    ),
    ("container_metrics_5s", "8 hours", None, "time", "system_id"),
    (
        "kube_metrics_1m",
        "2 days",
        Some("1 day"),
        "time",
        "system_id, namespace",
    ),
    (
        "kube_metrics_1h",
        "365 days",
        Some("14 days"),
        "bucket",
        "system_id, namespace",
    ),
    (
        "heartbeats",
        "365 days",
        Some("7 days"),
        "time",
        "monitor_id",
    ),
];

/// Chunk interval per tier — retention drops whole chunks, so this is the granularity at
/// which a tier can shrink. Short tiers need small chunks or they cannot shed anything.
const CHUNKS: &[(&str, &str)] = &[
    ("system_metrics_5s", "1 hour"),
    ("container_metrics_5s", "1 hour"),
    ("system_metrics_1m", "1 day"),
    ("system_metrics_1h", "7 days"),
    // 6h, not a day: retention drops WHOLE chunks, so a chunk only goes once all of its
    // data is past the window — a 1-day chunk against a 2-day window means holding up to
    // 3 days. On this tier that overshoot is over a gigabyte (measured: keep=2d, holding
    // 2.33d of a 1.34 GB/day table). Smaller chunks make the window mean what it says.
    ("kube_metrics_1m", "6 hours"),
    ("kube_metrics_1h", "7 days"),
    ("heartbeats", "1 day"),
];

pub async fn setup(config: &PgPool, data: &PgPool) {
    // Tables whose retention an admin changed in the UI — setup() must not reset those on
    // restart, only the ones nobody has touched.
    let overrides: Vec<String> =
        crate::settings::get(config, "retention_overrides", Vec::<String>::new()).await;
    let is_override = |t: &str| overrides.iter().any(|o| o == t);

    let mut stmts: Vec<String> = Vec::new();
    for (tbl, chunk) in CHUNKS {
        stmts.push(format!(
            "SELECT set_chunk_time_interval('{tbl}', INTERVAL '{chunk}')"
        ));
    }

    for (tbl, keep, compress_after, tcol, segment) in TIERS {
        if !is_override(tbl) {
            stmts.push(format!(
                "SELECT remove_retention_policy('{tbl}', if_exists => true)"
            ));
        }
        stmts.push(format!(
            "SELECT add_retention_policy('{tbl}', INTERVAL '{keep}', if_not_exists => true)"
        ));
        if let Some(after) = compress_after {
            stmts.push(format!(
                "ALTER TABLE {tbl} SET (timescaledb.compress, \
                    timescaledb.compress_segmentby = '{segment}', \
                    timescaledb.compress_orderby = '{tcol} DESC')"
            ));
            // Remove before add: `add_compression_policy` errors when one already exists
            // and every error here is swallowed, so without this a changed interval
            // silently never lands.
            stmts.push(format!(
                "SELECT remove_compression_policy('{tbl}', if_exists => true)"
            ));
            stmts.push(format!(
                "SELECT add_compression_policy('{tbl}', INTERVAL '{after}')"
            ));
        }
    }

    for s in &stmts {
        if let Err(e) = sqlx::query(s).execute(data).await {
            tracing::debug!(error = %e, "tier setup (ignored)");
        }
    }
    tracing::info!("metric tiers (raw / 1m / 1h) + retention + compression configured");
}

#[derive(Serialize)]
pub struct TableStat {
    pub name: String,
    pub size: String,
    /// On-disk size in bytes — lets the UI sort by size and colour the big tiers
    /// (the pretty `size` string can't be compared numerically).
    pub size_bytes: i64,
    pub rows: i64,
    /// What the table is for (config DB only; None for the data-DB tiers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Configured cleanup window in days for the time-growing log tables; None when
    /// the table isn't auto-pruned. Editable via POST /api/admin/config-retention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i32>,
    /// How many times smaller the compressed chunks are than they were. None when the
    /// tier has no compressed chunk yet. Worth showing: compression silently did nothing
    /// at all until 3.3.1, and nothing on the page would have revealed that.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_ratio: Option<f64>,
}

#[derive(Serialize)]
pub struct RetentionTier {
    pub table: String,
    pub label: String,
    /// "hours" for the raw realtime tier, "days" for the downsampled tiers.
    pub unit: String,
    pub value: Option<i64>,
    /// Age of the OLDEST row actually present, in days. The policy above says what the
    /// hub intends to keep; this says what it really has, and the two are not the same
    /// thing — cap eviction deletes data newer than a tier's window, and a rollup chain
    /// that gets rebuilt can only refill from the tier below it. Both were happening
    /// here unnoticed (14-day k8s retention holding 2.3 days, 365-day host rollup
    /// holding 44), because the UI only ever showed the intention.
    pub oldest_days: Option<f64>,
}

/// The raw realtime tiers (system + container) are managed in hours; the
/// downsampled rollups + heartbeats in days.
fn unit_for(table: &str) -> &'static str {
    if table.ends_with("_5s") {
        "hours"
    } else {
        "days"
    }
}

/// A plain relational DB's stats (used for the config DB — no hypertables/retention).
#[derive(Serialize)]
pub struct DbStats {
    pub db_size: String,
    /// Raw size so the UI can pick its own units. `pg_size_pretty` only switches to the
    /// next unit at 10x, so it reports "7968 MB" where a reader expects "7.8 GB".
    pub db_size_bytes: i64,
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
    pub db_size_bytes: i64,
    pub tables: Vec<TableStat>,
    pub retention: Vec<RetentionTier>,
    pub cap: CapStatus,
}

async fn hypertable_stat(data: &PgPool, name: &str, label: &str) -> TableStat {
    let size: Option<(i64, String)> =
        sqlx::query_as("SELECT hypertable_size($1), pg_size_pretty(hypertable_size($1))")
            .bind(name)
            .fetch_optional(data)
            .await
            .ok()
            .flatten();
    // NEVER `count(*)` here. These hypertables hold tens of millions of rows and the
    // rollups are continuous aggregates (a real-time CAgg count also re-aggregates the
    // uncovered raw tail) — 14 of those in one page load pinned every Postgres core.
    // `approximate_row_count()` reads chunk statistics instead: O(#chunks), no table scan.
    let rows: Option<(i64,)> = sqlx::query_as("SELECT approximate_row_count(($1::text)::regclass)")
        .bind(name)
        .fetch_optional(data)
        .await
        .ok()
        .flatten();
    let (size_bytes, size) = size.unwrap_or_else(|| (0, "—".into()));
    TableStat {
        name: label.to_string(),
        size,
        size_bytes,
        rows: rows.map(|(r,)| r).unwrap_or(0),
        note: None,
        retention_days: None,
        compression_ratio: compression_ratio(data, name).await,
    }
}

/// Ratio of pre- to post-compression bytes across a tier's compressed chunks. `None`
/// when nothing is compressed yet (or the relation isn't a compressed hypertable), which
/// is itself the useful signal — a tier that should be compressing and shows nothing is
/// a tier whose policy never ran.
async fn compression_ratio(data: &PgPool, table: &str) -> Option<f64> {
    let row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT (sum(before_compression_total_bytes)::float8 \
                 / nullif(sum(after_compression_total_bytes), 0))::float8 \
         FROM chunk_compression_stats($1)",
    )
    .bind(table)
    .fetch_optional(data)
    .await
    .ok()
    .flatten();
    row.and_then(|(r,)| r).filter(|r| r.is_finite() && *r > 1.0)
}

/// Reads a retention policy's `drop_after` interval for a hypertable, expressed
/// in the tier's unit (hours for the raw tier, days otherwise).
async fn retention_value(data: &PgPool, table: &str) -> Option<i64> {
    let divisor = if unit_for(table) == "hours" {
        3600
    } else {
        86400
    };
    // A rollup is a continuous aggregate: TimescaleDB records its retention job against
    // the MATERIALIZATION hypertable (`_materialized_hypertable_7`), never the view name —
    // so matching only on the view left every rollup's "Keep for" box blank in the UI even
    // though the policy existed. Resolve the view to its materialization table as well.
    let row: Option<(Option<i64>,)> = sqlx::query_as(&format!(
        "SELECT (EXTRACT(EPOCH FROM (j.config->>'drop_after')::interval) / {divisor})::bigint \
         FROM timescaledb_information.jobs j \
         WHERE j.proc_name = 'policy_retention' AND (j.hypertable_name = $1 \
           OR j.hypertable_name = (SELECT c.materialization_hypertable_name \
                FROM timescaledb_information.continuous_aggregates c WHERE c.view_name = $1))"
    ))
    .bind(table)
    .fetch_optional(data)
    .await
    .ok()
    .flatten();
    row.and_then(|(d,)| d)
}

/// Time column of a tier: the rollup tiers bucket into `bucket`, everything else
/// timestamps into `time`.
fn time_col(table: &str) -> &'static str {
    // `kube_metrics_1m` is the detail table the agent writes into, so despite its suffix
    // it timestamps samples in `time`; only tiers this hub rolls up have `bucket`.
    match table {
        "system_metrics_1m" | "system_metrics_1h" | "kube_metrics_1h" => "bucket",
        _ => "time",
    }
}

/// How old the oldest row in a tier is, in days. `min()` on the partitioning column is
/// the one unbounded aggregate that is safe on a hypertable: TimescaleDB walks chunks in
/// time order and stops at the first hit, so it costs one index probe rather than the
/// full scan that an unbounded `max(time)` per id would (see the query conventions).
async fn oldest_days(data: &PgPool, table: &str) -> Option<f64> {
    let tcol = time_col(table);
    let row: Option<(Option<f64>,)> = sqlx::query_as(&format!(
        "SELECT EXTRACT(EPOCH FROM (now() - min({tcol})))::float8 / 86400 FROM {table}"
    ))
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
        ("system_metrics_5s", "Hosts · 5-second"),
        ("system_metrics_1m", "Hosts · 1-minute"),
        ("system_metrics_1h", "Hosts · 1-hour"),
        ("kube_metrics_1m", "Kubernetes · 1-minute (detail)"),
        ("kube_metrics_1h", "Kubernetes · 1-hour"),
        ("container_metrics_5s", "Docker · 5-second"),
        ("heartbeats", "Service checks"),
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
            oldest_days: oldest_days(data, table).await,
        });
    }

    DataDbStats {
        db_size_bytes: db_size_bytes(data).await,
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

/// Per-table stats for a plain relational DB (the config DB). Small tables get an exact
/// `COUNT(*)` — the planner's `reltuples` estimate reads 0 (or -1) for any table not yet
/// ANALYZEd (e.g. users/workspaces with low write volume), which looks like the table is
/// empty when it isn't. Anything past `EXACT_COUNT_MAX_BYTES` (audit/transcript tables can
/// reach gigabytes) falls back to the estimate rather than scanning it on every page view.
pub async fn config_stats(config: &PgPool) -> DbStats {
    let db_size = sqlx::query_as::<_, (String,)>(
        "SELECT pg_size_pretty(pg_database_size(current_database()))",
    )
    .fetch_one(config)
    .await
    .map(|(s,)| s)
    .unwrap_or_else(|_| "—".into());
    // Table name + exact on-disk size from the catalog, biggest first.
    let metas: Vec<(String, i64, String, f32)> = sqlx::query_as(
        "SELECT c.relname, pg_total_relation_size(c.oid), pg_size_pretty(pg_total_relation_size(c.oid)), c.reltuples \
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
    for (name, size_bytes, size, reltuples) in metas {
        let rows = if size_bytes <= EXACT_COUNT_MAX_BYTES {
            // Names come from pg_catalog (our own schema); double-quote defensively.
            sqlx::query_as::<_, (i64,)>(&format!(
                "SELECT count(*) FROM \"{}\"",
                name.replace('"', "\"\"")
            ))
            .fetch_one(config)
            .await
            .map(|(n,)| n)
            .unwrap_or(0)
        } else {
            // reltuples is -1 when never analyzed; show 0 rather than a negative count.
            reltuples.max(0.0) as i64
        };
        let retention_days = retention.get(name.as_str()).copied();
        let note = config_table_note(&name).map(String::from);
        tables.push(TableStat {
            name,
            size,
            size_bytes,
            rows,
            note,
            retention_days,
            // The config DB is plain Postgres — no hypertables, so no compression.
            compression_ratio: None,
        });
    }
    DbStats {
        db_size_bytes: db_size_bytes(config).await,
        db_size,
        tables,
    }
}

/// Above this on-disk size a config table's row count comes from the planner estimate
/// instead of a full `count(*)` scan (the Data & retention page is not worth a seq scan).
const EXACT_COUNT_MAX_BYTES: i64 = 64 * 1024 * 1024;

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

/// Every Data-DB table keyed by `system_id`, for cleanup when a system is deleted.
/// A fixed allowlist — never interpolate a caller-supplied name into SQL.
const SYSTEM_TABLES: &[&str] = &[
    "system_metrics_5s",
    "system_metrics_1m",
    "system_metrics_1h",
    "container_metrics_5s",
    "kube_metrics_1m",
    "kube_metrics_1h",
];

/// Delete every metric belonging to a system. Best-effort: errors are logged, not
/// returned, because the system itself is already gone from the config DB and anything
/// left behind still expires on its normal schedule.
pub async fn purge_system(data: &PgPool, id: uuid::Uuid) {
    for t in SYSTEM_TABLES {
        if let Err(e) = sqlx::query(&format!("DELETE FROM {t} WHERE system_id = $1"))
            .bind(id)
            .execute(data)
            .await
        {
            tracing::warn!(error = %e, table = t, %id, "purging metrics for a deleted system");
        }
    }
}

/// Delete every heartbeat belonging to a monitor. Best-effort, as above.
pub async fn purge_monitor(data: &PgPool, id: uuid::Uuid) {
    if let Err(e) = sqlx::query("DELETE FROM heartbeats WHERE monitor_id = $1")
        .bind(id)
        .execute(data)
        .await
    {
        tracing::warn!(error = %e, %id, "purging heartbeats for a deleted monitor");
    }
}

/// Current cap config (from the config DB) + live Data-DB usage.
pub async fn cap_status(config: &PgPool, data: &PgPool) -> CapStatus {
    let limit_bytes = crate::settings::get(config, "data_cap_limit_bytes", DEFAULT_CAP_BYTES).await;
    let enabled = crate::settings::get(config, "data_cap_enabled", true).await;
    CapStatus {
        limit_bytes,
        used_bytes: db_size_bytes(data).await,
        enabled,
    }
}

/// Default Data-DB cap: 20 GiB, and eviction is ON by default (see
/// migrations/config/0002). An install with no ceiling fills its volume and takes
/// Postgres down with it, so the safe default is a cap that is enforced.
const DEFAULT_CAP_BYTES: i64 = 20 * 1024 * 1024 * 1024;

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
/// Data DB exceeds it, evict until back under (or no progress).
pub fn spawn_enforce(config: PgPool, data: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            enforce_cap(&config, &data).await;
        }
    });
}

/// Tiers eviction leaves alone while any other tier still has a chunk to drop. These are
/// the small, slow, long-horizon series — the ones whose whole value is that they go back
/// far — and they are cheap enough that evicting them buys almost no space. Everything
/// else (raw samples, short rollups) is regenerable detail and is fair game.
const PROTECTED_TIERS: &[&str] = &["system_metrics_1h", "kube_metrics_1h", "heartbeats"];

/// Oldest droppable chunk of the largest hypertable, skipping `protected`. Returns the
/// hypertable name and the boundary to drop below. Continuous-aggregate rollups live
/// under `_timescaledb_internal` and are dropped via their view, not by chunk name, so
/// they are not candidates here (they're small anyway).
async fn evict_target(data: &PgPool, protected: &[&str]) -> Option<(String, DateTime<Utc>)> {
    sqlx::query_as(
        "SELECT c.hypertable_name, \
                (SELECT ch.range_end FROM timescaledb_information.chunks ch \
                 WHERE ch.hypertable_schema = 'public' \
                   AND ch.hypertable_name = c.hypertable_name \
                   AND ch.range_end IS NOT NULL \
                 ORDER BY ch.range_end ASC LIMIT 1) \
         FROM timescaledb_information.hypertables c \
         WHERE c.hypertable_schema = 'public' AND c.hypertable_name <> ALL($1) \
         ORDER BY hypertable_size(format('%I.%I', c.hypertable_schema, c.hypertable_name)::regclass) DESC NULLS LAST \
         LIMIT 1",
    )
    .bind(protected.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    .fetch_optional(data)
    .await
    .ok()
    .flatten()
}

/// Upper bound on chunk drops per enforcement pass — a runaway-loop backstop, set far
/// above any realistic need (e.g. a year of daily heartbeat chunks + several tiers).
const MAX_DROPS_PER_PASS: u32 = 2000;

/// Outcome of one enforcement pass (also returned by the manual "evict now" endpoint).
#[derive(Serialize)]
pub struct EvictionResult {
    pub enabled: bool,
    pub limit_bytes: i64,
    /// DB size after the pass (== before, when nothing was evicted).
    pub used_bytes: i64,
    pub freed_bytes: i64,
    pub dropped_chunks: u32,
}

/// One enforcement pass. Reads the cap from the config DB; if enabled and the Data DB is
/// over its limit, reclaims space **from the largest hypertable first** — dropping that
/// tier's oldest chunks — and re-evaluates which tier is largest each round, until back
/// under the limit, nothing more can be dropped, or the safety cap is hit. Never touches
/// the config DB; only ever calls `drop_chunks` on the Data DB's hypertables.
///
/// Largest-first (not globally-oldest-first) is deliberate: one runaway tier is usually
/// the whole overage (e.g. per-container k8s stats), and globally-oldest would instead
/// delete a year of tiny, valuable heartbeat/rollup chunks while barely denting the DB.
///
/// Progress is judged by **how many chunks `drop_chunks` actually removed**, NOT by a
/// before/after `pg_database_size` comparison: under a fast ingest cadence, concurrent
/// writes can grow the DB between the two size reads faster than a small chunk frees,
/// which made the old size-delta guard `break` after ~one drop and never converge.
pub async fn enforce_cap(config: &PgPool, data: &PgPool) -> EvictionResult {
    let limit = crate::settings::get(config, "data_cap_limit_bytes", DEFAULT_CAP_BYTES).await;
    let enabled = crate::settings::get(config, "data_cap_enabled", true).await;
    let start = db_size_bytes(data).await;
    let mut used = start;
    let mut dropped_total: u32 = 0;
    // table -> (chunks dropped, newest boundary cut away)
    let mut per_table: std::collections::BTreeMap<String, (u32, DateTime<Utc>)> =
        std::collections::BTreeMap::new();
    // Being over the cap and doing nothing used to be completely silent, which reads as
    // "auto-delete is broken". Say why: the cap is off, or nothing was droppable.
    if !enabled && start > limit {
        tracing::warn!(
            used = %human_bytes(start),
            limit = %human_bytes(limit),
            "data cap: Data DB is over the configured limit but the cap is DISABLED — no eviction (enable it on Data & retention)"
        );
    }
    if enabled && start > limit {
        while used > limit {
            if dropped_total >= MAX_DROPS_PER_PASS {
                tracing::warn!(
                    dropped_total,
                    "data cap: hit the per-pass drop cap while still over limit — will resume next pass"
                );
                break;
            }
            // Oldest chunk of the LARGEST real (public) hypertable. Continuous-aggregate
            // rollups live under `_timescaledb_internal` and are dropped via their view,
            // not by chunk name, so we skip them here (they're small anyway).
            // Largest SACRIFICIAL tier first; the long-term tiers are only considered
            // once nothing else has a chunk left to give. Largest-first alone used to be
            // enough by luck — the long tiers are tiny today — but luck is not a
            // guarantee: let a year of hourly rollup actually accumulate and it becomes
            // the biggest table, at which point the plain largest-first rule would start
            // eating the one history worth keeping.
            let mut target = evict_target(data, PROTECTED_TIERS).await;
            if target.is_none() {
                tracing::warn!(
                    "data cap: every sacrificial tier is exhausted — falling back to the \
                     long-term tiers (raise the cap or shorten a retention window)"
                );
                target = evict_target(data, &[]).await;
            }
            let Some((ht, range_end)) = target else {
                break; // no hypertable / no droppable chunk left
            };
            // drop_chunks returns one row per chunk it removed — count them for progress.
            let dropped: Result<Vec<(String,)>, _> =
                sqlx::query_as("SELECT drop_chunks(format('%I', $1), older_than => $2)::text")
                    .bind(&ht)
                    .bind(range_end)
                    .fetch_all(data)
                    .await;
            match dropped {
                Ok(rows) if !rows.is_empty() => {
                    dropped_total += rows.len() as u32;
                    // Remember WHICH tier lost data and up to when. "freed 4.7 GB
                    // (1 chunks)" repeated nightly for three weeks never once said that
                    // every one of those chunks was a day of cluster history, so nobody
                    // connected the log to the charts that were visibly cut short.
                    let e = per_table.entry(ht.clone()).or_insert((0, range_end));
                    e.0 += rows.len() as u32;
                    e.1 = e.1.max(range_end);
                }
                Ok(_) => break, // couldn't drop the chosen chunk — avoid a tight loop
                Err(e) => {
                    tracing::warn!(error = %e, hypertable = %ht, "data cap: drop_chunks failed");
                    break;
                }
            }
            used = db_size_bytes(data).await;
        }
    }
    let freed = (start - used).max(0);
    if enabled && used > limit && dropped_total == 0 {
        tracing::warn!(
            used = %human_bytes(used),
            limit = %human_bytes(limit),
            "data cap: over the limit but no chunk could be dropped (only the newest chunk left, \
             or drop_chunks failed) — lower a tier's retention or raise the cap"
        );
    }
    if freed > 0 {
        let detail = per_table
            .iter()
            .map(|(t, (n, upto))| {
                format!(
                    "{t} ({n} chunks, deleted everything before {})",
                    upto.to_rfc3339()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let msg = format!(
            "data cap eviction: freed {} ({} chunks), now {} / limit {} — {}",
            human_bytes(freed),
            dropped_total,
            human_bytes(used),
            human_bytes(limit),
            detail
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
    EvictionResult {
        enabled,
        limit_bytes: limit,
        used_bytes: used,
        freed_bytes: freed,
        dropped_chunks: dropped_total,
    }
}

/// Allowlist of tables whose retention may be changed from the UI.
const RETENTION_TABLES: &[&str] = &[
    "system_metrics_5s",
    "system_metrics_1m",
    "system_metrics_1h",
    "container_metrics_5s",
    "kube_metrics_1m",
    "kube_metrics_1h",
    "heartbeats",
];

/// `value` is interpreted in the tier's unit (hours for the raw tier, days else).
/// Records the table as a user override (config DB) so `setup()` won't reset it to
/// the built-in default on the next restart.
pub async fn set_retention(
    config: &PgPool,
    data: &PgPool,
    table: &str,
    value: i64,
) -> Result<(), String> {
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
    // Remember this as an admin override (best-effort — the policy change already took).
    let mut overrides: Vec<String> =
        crate::settings::get(config, "retention_overrides", Vec::<String>::new()).await;
    if !overrides.iter().any(|t| t == table) {
        overrides.push(table.to_string());
        let _ = crate::settings::set(config, "retention_overrides", &overrides).await;
    }
    Ok(())
}
