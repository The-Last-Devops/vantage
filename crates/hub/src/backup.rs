//! Backup / restore. A snapshot is a JSON document of the **config** DB tables
//! (and optionally the **metrics** hypertables), gzipped. You can download it,
//! upload one to restore, or push/list/restore against an S3-compatible bucket.
//!
//! Restore is schema-agnostic: each table is dumped via `to_jsonb(row)` and
//! reloaded via `jsonb_populate_recordset(NULL::table, $1)`, so it tracks the
//! schema automatically. Config tables are restored inside a transaction in
//! FK-safe order (delete reverse, insert forward).

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

use crate::auth::CurrentUser;
use crate::AppState;

/// Config tables in FK-safe (parent → child) order. Transient tables (sessions,
/// audit_log, monitor_debug) are intentionally excluded.
const CONFIG_TABLES: &[&str] = &[
    "users",
    "namespaces",
    "memberships",
    "api_keys",
    "systems",
    "monitors",
    "channels",
    "alerts",
    "alert_state",
    "status_pages",
];
/// Append-only metrics hypertables (rollups are continuous aggregates — they
/// rematerialize from these, so they're not dumped).
const DATA_TABLES: &[&str] = &["system_metrics", "container_metrics", "heartbeats"];

fn admin(user: &CurrentUser) -> Result<(), StatusCode> {
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
        "format": "last-monitor-backup",
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

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(bytes);
    e.finish().unwrap_or_default()
}

fn maybe_gunzip(bytes: &[u8]) -> Vec<u8> {
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
    if snap.get("format").and_then(|v| v.as_str()) != Some("last-monitor-backup") {
        return Err("not a last-monitor backup file".into());
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

/// GET /api/admin/backup?metrics= — download a gzipped snapshot.
pub async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<BackupQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    admin(&user)?;
    let snap = build_snapshot(&state, q.metrics).await.map_err(|e| {
        tracing::error!(error = %e, "backup build");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let body = gzip(&serde_json::to_vec(&snap).unwrap_or_default());
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let name = format!("last-monitor-backup-{stamp}.json.gz");
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        body,
    ))
}

/// POST /api/admin/restore — restore from an uploaded snapshot (gz or plain JSON).
pub async fn restore(
    State(state): State<AppState>,
    user: CurrentUser,
    body: Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    let raw = maybe_gunzip(&body);
    let snap: Value = serde_json::from_slice(&raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid backup: {e}")))?;
    restore_snapshot(&state, &snap)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- S3 / S3-compatible ----------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct S3Config {
    pub endpoint: String, // e.g. https://s3.us-east-1.amazonaws.com or http://minio:9000
    pub region: String,   // e.g. us-east-1
    pub bucket: String,
    pub access_key: String,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub prefix: String, // optional key prefix, e.g. "last-monitor/"
}

async fn load_s3(state: &AppState) -> Result<S3Config, String> {
    let row: Option<(Value,)> = sqlx::query_as("SELECT s3 FROM app_settings WHERE id = 1")
        .fetch_optional(&state.config)
        .await
        .map_err(|e| e.to_string())?;
    let v = row.map(|(v,)| v).unwrap_or(Value::Null);
    if v.is_null() {
        return Err("S3 is not configured".into());
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}
fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
/// RFC3986 encode a path, keeping '/'. S3 keys are mostly safe chars.
fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if ok || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Sign + send one S3 request (path-style). `query` must be canonical (sorted,
/// encoded) or empty. Returns the response on 2xx, else an error with the body.
async fn s3_request(
    cfg: &S3Config,
    method: &str,
    key: &str,
    query: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response, String> {
    let url = reqwest::Url::parse(cfg.endpoint.trim_end_matches('/'))
        .map_err(|e| format!("bad endpoint: {e}"))?;
    let host = url.host_str().ok_or("endpoint has no host")?.to_string();
    let host_hdr = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.clone(),
    };
    // path-style: /<bucket>/<key>
    let canonical_uri = if key.is_empty() {
        format!("/{}", uri_encode(&cfg.bucket, false))
    } else {
        format!(
            "/{}/{}",
            uri_encode(&cfg.bucket, false),
            uri_encode(key, true)
        )
    };
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let datestamp = now.format("%Y%m%d").to_string();
    let payload_hash = sha256_hex(&body);
    let region = if cfg.region.is_empty() {
        "us-east-1"
    } else {
        &cfg.region
    };

    let canonical_headers =
        format!("host:{host_hdr}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{datestamp}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(
        format!("AWS4{}", cfg.secret_key).as_bytes(),
        datestamp.as_bytes(),
    );
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        cfg.access_key
    );

    let full = if query.is_empty() {
        format!("{}{canonical_uri}", url.origin().ascii_serialization())
    } else {
        format!(
            "{}{canonical_uri}?{query}",
            url.origin().ascii_serialization()
        )
    };
    let client = reqwest::Client::new();
    let req = client
        .request(
            method.parse().map_err(|_| "bad method")?,
            reqwest::Url::parse(&full).map_err(|e| e.to_string())?,
        )
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", &payload_hash)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .body(body);
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(resp)
    } else {
        let code = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        Err(format!("S3 {code}: {txt}"))
    }
}

fn s3_key(cfg: &S3Config, name: &str) -> String {
    let p = cfg.prefix.trim_matches('/');
    if p.is_empty() {
        format!("backups/{name}")
    } else {
        format!("{p}/backups/{name}")
    }
}

/// GET /api/admin/backup/s3 — current S3 settings (secret redacted).
pub async fn s3_get(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, StatusCode> {
    admin(&user)?;
    match load_s3(&state).await {
        Ok(c) => Ok(Json(json!({
            "configured": true,
            "endpoint": c.endpoint, "region": c.region, "bucket": c.bucket,
            "access_key": c.access_key, "prefix": c.prefix,
            "secret_set": !c.secret_key.is_empty(),
        }))),
        Err(_) => Ok(Json(json!({ "configured": false }))),
    }
}

/// PUT /api/admin/backup/s3 — save S3 settings (keep existing secret if omitted).
pub async fn s3_put(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(mut cfg): Json<S3Config>,
) -> Result<StatusCode, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    if cfg.secret_key.is_empty() {
        if let Ok(existing) = load_s3(&state).await {
            cfg.secret_key = existing.secret_key;
        }
    }
    let v = serde_json::to_value(&cfg).unwrap();
    sqlx::query(
        "INSERT INTO app_settings (id, s3) VALUES (1, $1) \
         ON CONFLICT (id) DO UPDATE SET s3 = $1",
    )
    .bind(v)
    .execute(&state.config)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/admin/backup/s3/test — verify creds by listing the bucket.
pub async fn s3_test(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<StatusCode, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    let cfg = load_s3(&state)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    s3_request(&cfg, "GET", "", "list-type=2&max-keys=1", Vec::new())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/admin/backup/s3/upload?metrics= — snapshot → gzip → PUT to the bucket.
pub async fn s3_upload(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<BackupQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    let cfg = load_s3(&state)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let snap = build_snapshot(&state, q.metrics)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let body = gzip(&serde_json::to_vec(&snap).unwrap_or_default());
    let name = format!(
        "last-monitor-backup-{}.json.gz",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let key = s3_key(&cfg, &name);
    s3_request(&cfg, "PUT", &key, "", body)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!({ "ok": true, "key": key })))
}

/// GET /api/admin/backup/s3/list — backup object keys in the bucket.
pub async fn s3_list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    let cfg = load_s3(&state)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let prefix = s3_key(&cfg, "");
    let query = format!("list-type=2&prefix={}", uri_encode(&prefix, true));
    let resp = s3_request(&cfg, "GET", "", &query, Vec::new())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    let xml = resp.text().await.unwrap_or_default();
    // crude XML extraction of <Key>…</Key> — good enough for ListObjectsV2.
    let mut keys: Vec<&str> = xml
        .split("<Key>")
        .skip(1)
        .filter_map(|s| s.split("</Key>").next())
        .collect();
    keys.sort_unstable();
    keys.reverse(); // newest (timestamped) first
    Ok(Json(json!({ "keys": keys })))
}

#[derive(Deserialize)]
pub struct S3RestoreReq {
    pub key: String,
}

/// POST /api/admin/backup/s3/restore — fetch a snapshot from the bucket + restore.
pub async fn s3_restore(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<S3RestoreReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    admin(&user).map_err(|s| (s, "admin only".into()))?;
    let cfg = load_s3(&state)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let resp = s3_request(&cfg, "GET", &req.key, "", Vec::new())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let raw = maybe_gunzip(&bytes);
    let snap: Value = serde_json::from_slice(&raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid backup: {e}")))?;
    restore_snapshot(&state, &snap)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(StatusCode::NO_CONTENT)
}
