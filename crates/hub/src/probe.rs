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
// Honest, identifying defaults so WAFs don't reject a request with no UA/Accept
// (e.g. a bare 406). Both are overridable per monitor via the headers config.
const DEFAULT_UA: &str = concat!(
    "last-monitor/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/the-last-devops/last-monitor)"
);
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
                                // Push monitors aren't probed; we just check staleness — the
                                // "up" beats arrive via /pub/push/<token>.
                                if m.kind == "push" {
                                    let last: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
                                        "SELECT time FROM heartbeats WHERE monitor_id = $1 ORDER BY time DESC LIMIT 1",
                                    )
                                    .bind(m.id)
                                    .fetch_optional(&data)
                                    .await
                                    .ok()
                                    .flatten();
                                    let stale = match last {
                                        Some((t,)) => {
                                            (chrono::Utc::now() - t).num_seconds().max(0) as u64
                                                > m.interval.as_secs()
                                        }
                                        None => true,
                                    };
                                    if stale {
                                        let beat = Beat {
                                            up: false,
                                            latency_ms: None,
                                            status_code: None,
                                            message: Some(
                                                "no push received within interval".into(),
                                            ),
                                            debug: None,
                                        };
                                        let _ = write_beat(&data, m.id, &beat).await;
                                    }
                                    return;
                                }
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
                                let retries = cfg_u64(&m.config, "retries", 1);
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
        "postgres" => probe_postgres(m, start).await,
        "mysql" => probe_mysql(m, start).await,
        "mongodb" => probe_mongodb(m, start).await,
        "redis" => probe_redis(m, start).await,
        "rabbitmq" => probe_rabbitmq(m, start).await,
        "dns" => probe_dns(m, start).await,
        "tls" => probe_tls(m, start).await,
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
                // Send the real value, but never persist credential-bearing headers
                // to monitor_debug (viewers can read that).
                let sensitive = matches!(
                    k.to_ascii_lowercase().as_str(),
                    "authorization"
                        | "proxy-authorization"
                        | "cookie"
                        | "set-cookie"
                        | "x-api-key"
                        | "api-key"
                        | "x-auth-token"
                        | "x-token"
                        | "token"
                );
                req_headers.insert(k.clone(), json!(if sensitive { "****" } else { vs }));
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

/// Build an "up" beat for the simple connect-style checks.
fn ok_beat(start: Instant, target: &str, msg: Option<String>) -> Beat {
    Beat {
        up: true,
        latency_ms: Some(start.elapsed().as_millis() as i32),
        status_code: None,
        message: msg,
        debug: Some(json!({ "target": target })),
    }
}
fn err_beat(start: Instant, target: &str, msg: String) -> Beat {
    Beat {
        up: false,
        latency_ms: Some(start.elapsed().as_millis() as i32),
        status_code: None,
        message: Some(truncate(&msg, 200)),
        debug: Some(json!({ "target": target, "error": msg })),
    }
}

/// PostgreSQL: connect (target is a `postgres://…` URL) and run `SELECT 1`.
async fn probe_postgres(m: &Monitor, start: Instant) -> Beat {
    use sqlx::Connection;
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let fut = async {
        let mut conn = sqlx::postgres::PgConnection::connect(&m.target).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        let _ = conn.close().await;
        Ok::<(), sqlx::Error>(())
    };
    match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(())) => ok_beat(start, &m.target, None),
        Ok(Err(e)) => err_beat(start, &m.target, e.to_string()),
        Err(_) => err_beat(start, &m.target, "connect timeout".into()),
    }
}

/// Redis: TCP connect to host:port, optional AUTH, then PING → expect +PONG.
async fn probe_redis(m: &Monitor, start: Instant) -> Beat {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let pass = cfg_str(&m.config, "password").map(str::to_owned);
    let fut = async {
        let mut s = tokio::net::TcpStream::connect(&m.target).await?;
        if let Some(p) = pass {
            s.write_all(format!("AUTH {p}\r\n").as_bytes()).await?;
        }
        s.write_all(b"PING\r\n").await?;
        let mut buf = [0u8; 128];
        let n = s.read(&mut buf).await?;
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&buf[..n]).into_owned())
    };
    match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(resp)) if resp.contains("PONG") => ok_beat(start, &m.target, None),
        Ok(Ok(resp)) => err_beat(
            start,
            &m.target,
            format!("unexpected reply: {}", resp.trim()),
        ),
        Ok(Err(e)) => err_beat(start, &m.target, e.to_string()),
        Err(_) => err_beat(start, &m.target, "connect timeout".into()),
    }
}

/// RabbitMQ: TCP connect + send the AMQP 0-9-1 protocol header; a live broker
/// replies (Connection.Start or a version header). No AMQP client needed.
async fn probe_rabbitmq(m: &Monitor, start: Instant) -> Beat {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let fut = async {
        let mut s = tokio::net::TcpStream::connect(&m.target).await?;
        s.write_all(b"AMQP\x00\x00\x09\x01").await?;
        let mut buf = [0u8; 16];
        let n = s.read(&mut buf).await?;
        Ok::<usize, std::io::Error>(n)
    };
    match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(n)) if n > 0 => ok_beat(start, &m.target, None),
        Ok(Ok(_)) => err_beat(start, &m.target, "no AMQP response".into()),
        Ok(Err(e)) => err_beat(start, &m.target, e.to_string()),
        Err(_) => err_beat(start, &m.target, "connect timeout".into()),
    }
}

/// DNS: resolve the target hostname; up if it resolves (optionally containing an
/// expected IP substring from config `expected_ip`).
async fn probe_dns(m: &Monitor, start: Instant) -> Beat {
    let exp = cfg_str(&m.config, "expected_ip");
    match tokio::net::lookup_host((m.target.as_str(), 0)).await {
        Ok(it) => {
            let ips: Vec<String> = it.map(|sa| sa.ip().to_string()).collect();
            if ips.is_empty() {
                return err_beat(start, &m.target, "no DNS records".into());
            }
            if let Some(e) = exp {
                if !ips.iter().any(|ip| ip.contains(e)) {
                    return err_beat(
                        start,
                        &m.target,
                        format!("expected {e}, got {}", ips.join(", ")),
                    );
                }
            }
            Beat {
                up: true,
                latency_ms: Some(start.elapsed().as_millis() as i32),
                status_code: None,
                message: Some(ips.join(", ")),
                debug: Some(json!({ "target": m.target, "resolved": ips })),
            }
        }
        Err(e) => err_beat(start, &m.target, e.to_string()),
    }
}

/// MySQL/MariaDB: connect (target is a `mysql://…` URL) and run `SELECT 1`.
async fn probe_mysql(m: &Monitor, start: Instant) -> Beat {
    use sqlx::Connection;
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let fut = async {
        let mut conn = sqlx::mysql::MySqlConnection::connect(&m.target).await?;
        sqlx::query("SELECT 1").execute(&mut conn).await?;
        let _ = conn.close().await;
        Ok::<(), sqlx::Error>(())
    };
    match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(())) => ok_beat(start, &m.target, None),
        Ok(Err(e)) => err_beat(start, &m.target, e.to_string()),
        Err(_) => err_beat(start, &m.target, "connect timeout".into()),
    }
}

/// MongoDB: connect (target is a `mongodb://…` URI) and run `{ping:1}`.
async fn probe_mongodb(m: &Monitor, start: Instant) -> Beat {
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let fut = async {
        let client = mongodb::Client::with_uri_str(&m.target).await?;
        client
            .database("admin")
            .run_command(mongodb::bson::doc! { "ping": 1 })
            .await?;
        Ok::<(), mongodb::error::Error>(())
    };
    match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(())) => ok_beat(start, &m.target, None),
        Ok(Err(e)) => err_beat(start, &m.target, e.to_string()),
        Err(_) => err_beat(start, &m.target, "connect timeout".into()),
    }
}

/// TLS certificate expiry: handshake to host:port (cert verification disabled so
/// we can read even an expired/self-signed cert) and report days until notAfter.
/// Down when expired or within `cert_warn_days` (default 14).
async fn probe_tls(m: &Monitor, start: Instant) -> Beat {
    use tokio_rustls::rustls;
    let to = cfg_u64(&m.config, "timeout_secs", 10).clamp(1, 60);
    let warn = cfg_u64(&m.config, "cert_warn_days", 14) as i64;
    let host = m
        .target
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(&m.target)
        .to_string();

    #[derive(Debug)]
    struct NoVerify(rustls::crypto::CryptoProvider);
    impl rustls::client::danger::ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _e: &rustls::pki_types::CertificateDer,
            _i: &[rustls::pki_types::CertificateDer],
            _s: &rustls::pki_types::ServerName,
            _o: &[u8],
            _n: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    let provider = rustls::crypto::ring::default_provider();
    let config = match rustls::ClientConfig::builder_with_provider(provider.clone().into())
        .with_safe_default_protocol_versions()
    {
        Ok(b) => b
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(NoVerify(provider)))
            .with_no_client_auth(),
        Err(e) => return err_beat(start, &m.target, e.to_string()),
    };
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let server_name = match rustls::pki_types::ServerName::try_from(host.clone()) {
        Ok(s) => s,
        Err(e) => return err_beat(start, &m.target, e.to_string()),
    };

    let fut = async {
        let tcp = tokio::net::TcpStream::connect(&m.target).await?;
        let tls = connector.connect(server_name, tcp).await?;
        let der = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|c| c.as_ref().to_vec());
        Ok::<Option<Vec<u8>>, std::io::Error>(der)
    };
    let der = match timeout(Duration::from_secs(to), fut).await {
        Ok(Ok(Some(d))) => d,
        Ok(Ok(None)) => return err_beat(start, &m.target, "no peer certificate".into()),
        Ok(Err(e)) => return err_beat(start, &m.target, e.to_string()),
        Err(_) => return err_beat(start, &m.target, "handshake timeout".into()),
    };
    let not_after = match x509_parser::parse_x509_certificate(&der) {
        Ok((_, cert)) => cert.validity().not_after.timestamp(),
        Err(e) => return err_beat(start, &m.target, format!("parse cert: {e}")),
    };
    let days = (not_after - chrono::Utc::now().timestamp()) / 86_400;
    let msg = format!("cert expires in {days} days");
    Beat {
        up: days > warn,
        latency_ms: Some(start.elapsed().as_millis() as i32),
        status_code: Some(days as i32),
        message: Some(msg),
        debug: Some(json!({ "target": m.target, "days_left": days, "warn_days": warn })),
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
