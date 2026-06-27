//! Per-check-kind probe implementations and the kind dispatcher.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::time::timeout;

use super::{
    cfg_bool, cfg_str, cfg_u64, down, err_beat, ok_beat, status_matches, truncate, Beat, Monitor,
    BODY_CAP, DEFAULT_ACCEPT, DEFAULT_UA,
};

pub(super) async fn probe(m: &Monitor) -> Beat {
    let start = Instant::now();
    // SSRF egress guard: reject internal/metadata targets before connecting.
    // `dns` is resolution-only (no outbound data connection), so it's exempt.
    // Resolve off the async worker — getaddrinfo can block.
    if m.kind != "dns" {
        let t = m.target.clone();
        match tokio::task::spawn_blocking(move || crate::net_guard::check_target(&t)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return down(&e.to_string()),
            Err(_) => return down("egress check failed"),
        }
    }
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

async fn probe_http(m: &Monitor, start: Instant) -> Beat {
    let c = &m.config;
    let to = cfg_u64(c, "timeout_secs", 15).clamp(1, 120);
    let max_redirects = cfg_u64(c, "max_redirects", 10) as usize;
    let redirect = if max_redirects == 0 {
        reqwest::redirect::Policy::none()
    } else {
        // Follow up to the limit, but re-check each hop so a redirect can't bounce
        // the request to an internal/metadata address (SSRF via redirect).
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= max_redirects {
                attempt.stop()
            } else if crate::net_guard::check_target(attempt.url().as_str()).is_err() {
                attempt.error("blocked redirect to internal/metadata address".to_string())
            } else {
                attempt.follow()
            }
        })
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
