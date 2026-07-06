//! Backup / restore. A snapshot is a JSON document of the **config** DB tables
//! (and optionally the **metrics** hypertables), gzipped. You can download it,
//! upload one to restore, or push/list/restore against an S3-compatible bucket.
//!
//! Restore is schema-agnostic: each table is dumped via `to_jsonb(row)` and
//! reloaded via `jsonb_populate_recordset(NULL::table, $1)`, so it tracks the
//! schema automatically. Config tables are restored inside a transaction in
//! FK-safe order (delete reverse, insert forward).
//!
//! Split into three concerns:
//! - [`local`] — build/restore a snapshot + the download/restore HTTP handlers.
//! - [`s3`] — S3-compatible signing and the push/list/restore-from-bucket handlers.
//! - [`schedule`] — the schedule settings handlers and the cron `spawn` loop.

mod local;
mod s3;
mod schedule;

pub use local::{download, restore};
pub use s3::{s3_get, s3_list, s3_put, s3_restore, s3_test, s3_upload};
pub use schedule::{schedule_get, schedule_put, spawn};

use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};

use crate::auth::CurrentUser;
use crate::AppState;
use axum::http::StatusCode;

/// Config tables in FK-safe (parent → child) order. Transient tables (sessions,
/// audit_log, monitor_debug) are intentionally excluded.
// Parent → child order (restore inserts in this order, deletes in reverse) so FKs
// hold. Auth artifacts (api_pats / ssh_keys / webauthn_credentials) MUST be here or a
// restore silently destroys every PAT, SSH key and passkey when `users` is replaced.
pub(crate) const CONFIG_TABLES: &[&str] = &[
    "users",
    "workspaces",
    "memberships",
    "api_pats",
    "ssh_keys",
    "webauthn_credentials",
    "api_keys",
    "systems",
    "monitors",
    "channels",
    "alerts",
    "alert_state",
    "status_pages",
    "settings",
];
/// Append-only metrics hypertables (rollups are continuous aggregates — they
/// rematerialize from these, so they're not dumped).
pub(crate) const DATA_TABLES: &[&str] = &["system_metrics", "container_metrics", "heartbeats"];

pub(crate) fn admin(user: &CurrentUser) -> Result<(), StatusCode> {
    if user.is_admin {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn dump_table(pool: &sqlx::PgPool, table: &str) -> Result<Value, String> {
    let (v,): (Value,) = sqlx::query_as(&format!(
        "SELECT coalesce(jsonb_agg(to_jsonb(t)), '[]'::jsonb) FROM \"{table}\" t"
    ))
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(v)
}

/// Build the snapshot document (optionally including the metrics hypertables).
pub async fn build_snapshot(state: &AppState, include_metrics: bool) -> Result<Value, String> {
    let mut config = Map::new();
    for t in CONFIG_TABLES {
        config.insert(t.to_string(), dump_table(&state.config, t).await?);
    }
    let mut out = json!({
        "format": "vantage-backup",
        "version": 1,
        "created_at": Utc::now().to_rfc3339(),
        "include_metrics": include_metrics,
        "config": config,
    });
    if include_metrics {
        let mut metrics = Map::new();
        for t in DATA_TABLES {
            metrics.insert(t.to_string(), dump_table(&state.data, t).await?);
        }
        out["metrics"] = Value::Object(metrics);
    }
    Ok(out)
}

pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(bytes);
    e.finish().unwrap_or_default()
}

pub(crate) fn maybe_gunzip(bytes: &[u8]) -> Vec<u8> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut d = flate2::read::GzDecoder::new(bytes);
        let mut out = Vec::new();
        if d.read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    bytes.to_vec()
}

async fn restore_into(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tables: &[&str],
    data: &Map<String, Value>,
) -> Result<(), String> {
    // delete child → parent (reverse), then insert parent → child (forward)
    for t in tables.iter().rev() {
        sqlx::query(&format!("DELETE FROM \"{t}\""))
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("clearing {t}: {e}"))?;
    }
    for t in tables {
        if let Some(rows) = data.get(*t) {
            sqlx::query(&format!(
                "INSERT INTO \"{t}\" SELECT * FROM jsonb_populate_recordset(NULL::\"{t}\", $1)"
            ))
            .bind(rows)
            .execute(&mut **tx)
            .await
            .map_err(|e| format!("restoring {t}: {e}"))?;
        }
    }
    Ok(())
}

/// Apply a snapshot. Config restore is one transaction; metrics another.
pub async fn restore_snapshot(state: &AppState, snap: &Value) -> Result<(), String> {
    if snap.get("format").and_then(|v| v.as_str()) != Some("vantage-backup") {
        return Err("not a vantage backup file".into());
    }
    let config = snap
        .get("config")
        .and_then(|v| v.as_object())
        .ok_or("snapshot has no config")?;
    let mut tx = state.config.begin().await.map_err(|e| e.to_string())?;
    restore_into(&mut tx, CONFIG_TABLES, config).await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    if let Some(metrics) = snap.get("metrics").and_then(|v| v.as_object()) {
        let mut tx = state.data.begin().await.map_err(|e| e.to_string())?;
        restore_into(&mut tx, DATA_TABLES, metrics).await?;
        tx.commit().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct BackupQuery {
    #[serde(default)]
    pub metrics: bool,
}
