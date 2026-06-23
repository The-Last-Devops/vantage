//! Service-check probe engine (the Uptime-Kuma half).
//!
//! A single background scheduler reloads enabled monitors from the config DB,
//! fires each one on its own interval, and writes a heartbeat row into the data DB.
//! It also stores the last successful and last failed request/response per monitor
//! (the `monitor_debug` table) so a failure like a bare 406 can be inspected.
//!
//! Per-monitor options live in the `config` JSONB (all optional):
//!   timeout_secs, retries, upside_down,
//!   method, headers{}, body, auth{type,username,password,token},
//!   accepted_status ("200-299,301"), max_redirects, ignore_tls,
//!   keyword, keyword_invert   (tags/description are metadata, ignored here)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sqlx::types::Json;
use tokio::time::timeout;
use uuid::Uuid;

use crate::AppState;

#[derive(Clone)]
struct Monitor {
    id: Uuid,
    kind: String,
    target: String,
    interval: Duration,
    config: Value,
}

struct Beat {
    up: bool,
    latency_ms: Option<i32>,
    status_code: Option<i32>,
    message: Option<String>,
    /// Rich request/response detail for the debug view (best-effort).
    debug: Option<Value>,
}

const TICK: Duration = Duration::from_secs(2);
const BODY_CAP: usize = 4096;
// Browser-ish defaults so WAFs don't reject a "bot" with no UA/Accept (e.g. 406).
// Both are overridable per monitor via the headers config.
const DEFAULT_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

fn cfg_u64(c: &Value, key: &str, default: u64) -> u64 {
    c.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
}
fn cfg_bool(c: &Value, key: &str) -> bool {
    c.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}
fn cfg_str<'a>(c: &'a Value, key: &str) -> Option<&'a str> {
    c.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Parse "200-299,301,400-403" → ranges. Empty/None means "any 2xx".
fn status_matches(spec: Option<&str>, code: u16) -> bool {
    let Some(spec) = spec else {
        return (200..300).contains(&code);
    };
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(lo), Ok(hi)) = (a.trim().parse::<u16>(), b.trim().parse::<u16>()) {
                if (lo..=hi).contains(&code) {
                    return true;
                }
            }
        } else if part.parse::<u16>() == Ok(code) {
            return true;
        }
    }
    false
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut last_run: HashMap<Uuid, Instant> = HashMap::new();
        let streaks: Arc<Mutex<HashMap<Uuid, u64>>> = Arc::new(Mutex::new(HashMap::new()));

        loop {
            match load_monitors(&state).await {
                Ok(monitors) => {
                    let now = Instant::now();
                    let live: std::collections::HashSet<Uuid> =
                        monitors.iter().map(|m| m.id).collect();
                    last_run.retain(|id, _| live.contains(id));
                    streaks.lock().unwrap().retain(|id, _| live.contains(id));

                    for m in monitors {
                        let due = match last_run.get(&m.id) {
                            Some(t) => now.duration_since(*t) >= m.interval,
                            None => true,
                        };
                        if due {
                            last_run.insert(m.id, now);
                            let data = state.data.clone();
                            let config = state.config.clone();
                            let streaks = streaks.clone();
                            tokio::spawn(async move {
                                let mut beat = probe(&m).await;
                                // The raw check result (before upside-down / retries) is what
                                // we classify the debug record by.
                                let raw_up = beat.up;
                                let debug = beat.debug.take();

                                if cfg_bool(&m.config, "upside_down") {
                                    beat.up = !beat.up;
                                    if !beat.up {
                                        beat.message = Some("up (inverted by upside-down)".into());
                                    }
                                }
                                let retries = cfg_u64(&m.config, "retries", 0);
                                let streak = {
                                    let mut g = streaks.lock().unwrap();
                                    let s = if beat.up {
                                        0
                                    } else {
                                        g.get(&m.id).copied().unwrap_or(0) + 1
                                    };
                                    g.insert(m.id, s);
                                    s
                                };
                                if !beat.up && streak <= retries {
                                    beat.up = true;
                                    beat.message = Some(format!(
                                        "{} (retry {}/{})",
                                        beat.message.as_deref().unwrap_or("check failed"),
                                        streak,
                                        retries
                                    ));
                                }
                                if let Err(e) = write_beat(&data, m.id, &beat).await {
                                    tracing::error!(error = %e, monitor = %m.id, "write heartbeat");
                                }
                                if let Some(detail) = debug {
                                    let outcome = if raw_up { "ok" } else { "err" };
                                    let _ = write_debug(&config, m.id, outcome, &detail).await;
                                }
                            });
                        }
                    }
                }
                Err(e) => tracing::error!(error = %e, "load monitors"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn load_monitors(state: &AppState) -> anyhow::Result<Vec<Monitor>> {
    let rows: Vec<(Uuid, String, String, i32, Json<Value>)> = sqlx::query_as(
        "SELECT id, kind::text, target, interval_secs, config \
         FROM monitors WHERE enabled = true",
    )
    .fetch_all(&state.config)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, kind, target, interval_secs, config)| Monitor {
            id,
            kind,
            target,
            interval: Duration::from_secs(interval_secs.max(1) as u64),
            config: config.0,
        })
        .collect())
}

async fn probe(m: &Monitor) -> Beat {
    let start = Instant::now();
    match m.kind.as_str() {
        "http" | "keyword" => probe_http(m, start).await,
        "tcp" => probe_tcp(m, start).await,
        "ping" => probe_ping(m).await,
        other => down(&format!("unsupported monitor kind: {other}")),
    }
}

async fn probe_ping(m: &Monitor) -> Beat {
    let to = cfg_u64(&m.config, "timeout_secs", 5).clamp(1, 60);
    let addr = match tokio::net::lookup_host((m.target.as_str(), 0)).await {
        Ok(mut it) => match it.next() {
            Some(sa) => sa.ip(),
            None => return down("no DNS result"),
        },
        Err(e) => return down(&e.to_string()),
    };
    let client = match surge_ping::Client::new(&surge_ping::Config::default()) {
        Ok(c) => c,
        Err(e) => return down(&format!("icmp socket: {e}")),
    };
    let id = surge_ping::PingIdentifier(rand::random());
    let mut pinger = client.pinger(addr, id).await;
    pinger.timeout(Duration::from_secs(to));
    match pinger.ping(surge_ping::PingSequence(0), &[0u8; 32]).await {
        Ok((_, rtt)) => Beat {
            up: true,
            latency_ms: Some(rtt.as_millis() as i32),
            status_code: None,
            message: None,
            debug: Some(json!({ "target": m.target, "resolved": addr.to_string() })),
        },
        Err(e) => {
            let mut b = down(&e.to_string());
            b.debug = Some(
                json!({ "target": m.target, "resolved": addr.to_string(), "error": e.to_string() }),
            );
            b
        }
    }
}

fn down(msg: &str) -> Beat {
    Beat {
        up: false,
        latency_ms: None,
        status_code: None,
        message: Some(truncate(msg, 200)),
        debug: None,
    }
}

async fn probe_http(m: &Monitor, start: Instant) -> Beat {
    let c = &m.config;
    let to = cfg_u64(c, "timeout_secs", 15).clamp(1, 120);
    let max_redirects = cfg_u64(c, "max_redirects", 10) as usize;
    let redirect = if max_redirects == 0 {
        reqwest::redirect::Policy::none()
    } else {
        reqwest::redirect::Policy::limited(max_redirects)
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(to))
        .danger_accept_invalid_certs(cfg_bool(c, "ignore_tls"))
        .redirect(redirect)
        .user_agent(DEFAULT_UA)
        .build()
    {
        Ok(c) => c,
        Err(e) => return down(&e.to_string()),
    };

    let method_s = cfg_str(c, "method").unwrap_or("GET").to_uppercase();
    let method = reqwest::Method::from_bytes(method_s.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut req = client.request(method, &m.target);
    let mut req_headers = serde_json::Map::new();
    if let Some(headers) = c.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(vs) = v.as_str() {
                req = req.header(k, vs);
                req_headers.insert(k.clone(), json!(vs));
            }
        }
    }
    if let Some(auth) = c.get("auth").and_then(|v| v.as_object()) {
        match auth.get("type").and_then(|v| v.as_str()) {
            Some("basic") => {
                req = req.basic_auth(
                    auth.get("username").and_then(|v| v.as_str()).unwrap_or(""),
                    auth.get("password").and_then(|v| v.as_str()),
                );
                req_headers.insert("Authorization".into(), json!("Basic ****"));
            }
            Some("bearer") => {
                req = req.bearer_auth(auth.get("token").and_then(|v| v.as_str()).unwrap_or(""));
                req_headers.insert("Authorization".into(), json!("Bearer ****"));
            }
            _ => {}
        }
    }
    if let Some(body) = cfg_str(c, "body") {
        req = req.body(body.to_owned());
    }
    // Browser-ish defaults (overridable above): reflect them in the debug record.
    let has = |name: &str| req_headers.keys().any(|k| k.eq_ignore_ascii_case(name));
    let (has_ua, has_accept) = (has("user-agent"), has("accept"));
    if !has_ua {
        req_headers.insert("User-Agent".into(), json!(DEFAULT_UA));
    }
    if !has_accept {
        req = req.header("Accept", DEFAULT_ACCEPT);
        req_headers.insert("Accept".into(), json!(DEFAULT_ACCEPT));
    }
    let req_dbg = json!({ "method": method_s, "url": m.target, "headers": req_headers });

    let accepted = cfg_str(c, "accepted_status");
    let keyword = cfg_str(c, "keyword").map(str::to_owned);
    let kw_invert = cfg_bool(c, "keyword_invert");

    match req.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let resp_headers: serde_json::Map<String, Value> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or(""))))
                .collect();
            // Always read the body (truncated) so the debug record is useful.
            let body = resp.text().await.unwrap_or_default();
            let body_snip = truncate(&body, BODY_CAP);

            let mut up = status_matches(accepted, code);
            let mut message = if up {
                None
            } else {
                Some(format!("unexpected status {code}"))
            };
            if let Some(kw) = &keyword {
                let found = body.contains(kw);
                if found == kw_invert {
                    up = false;
                    message = Some(if kw_invert {
                        format!("keyword '{kw}' present (should be absent)")
                    } else {
                        format!("keyword '{kw}' not found")
                    });
                }
            }
            Beat {
                up,
                latency_ms: Some(start.elapsed().as_millis() as i32),
                status_code: Some(code as i32),
                message,
                debug: Some(json!({
                    "request": req_dbg,
                    "response": { "status": code, "headers": resp_headers, "body": body_snip },
                    "latency_ms": start.elapsed().as_millis() as i32,
                })),
            }
        }
        Err(e) => Beat {
            up: false,
            latency_ms: Some(start.elapsed().as_millis() as i32),
            status_code: None,
            message: Some(truncate(&e.to_string(), 200)),
            debug: Some(json!({ "request": req_dbg, "error": e.to_string() })),
        },
    }
}

async fn probe_tcp(m: &Monitor, start: Instant) -> Beat {
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let connect = tokio::net::TcpStream::connect(&m.target);
    let dbg = |err: Option<String>| json!({ "target": m.target, "error": err });
    match timeout(Duration::from_secs(to), connect).await {
        Ok(Ok(_stream)) => Beat {
            up: true,
            latency_ms: Some(start.elapsed().as_millis() as i32),
            status_code: None,
            message: None,
            debug: Some(dbg(None)),
        },
        Ok(Err(e)) => Beat {
            up: false,
            latency_ms: Some(start.elapsed().as_millis() as i32),
            status_code: None,
            message: Some(truncate(&e.to_string(), 200)),
            debug: Some(dbg(Some(e.to_string()))),
        },
        Err(_) => {
            let mut b = down("connect timeout");
            b.debug = Some(dbg(Some("connect timeout".into())));
            b
        }
    }
}

async fn write_beat(data: &sqlx::PgPool, monitor_id: Uuid, beat: &Beat) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO heartbeats (time, monitor_id, up, latency_ms, status_code, message) \
         VALUES (now(), $1, $2, $3, $4, $5)",
    )
    .bind(monitor_id)
    .bind(beat.up)
    .bind(beat.latency_ms)
    .bind(beat.status_code)
    .bind(beat.message.as_deref())
    .execute(data)
    .await?;
    Ok(())
}

/// Upsert the last 'ok' / 'err' request-response detail for a monitor.
async fn write_debug(
    config: &sqlx::PgPool,
    monitor_id: Uuid,
    outcome: &str,
    detail: &Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO monitor_debug (monitor_id, outcome, detail, at) VALUES ($1, $2, $3, now()) \
         ON CONFLICT (monitor_id, outcome) DO UPDATE SET detail = EXCLUDED.detail, at = now()",
    )
    .bind(monitor_id)
    .bind(outcome)
    .bind(sqlx::types::Json(detail))
    .execute(config)
    .await?;
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
