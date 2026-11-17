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
//!   domain_expiry: warn_days, whois_server (see [`domain_expiry`])
//!
//! Split into two concerns:
//! - [`engine`] — the background scheduler ([`spawn`]), the immediate one-shot
//!   ([`check_once`]), monitor loading, and the heartbeat/debug writes.
//! - [`checks`] — the per-kind probe implementations ([`checks::probe`]).

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use uuid::Uuid;

mod checks;
mod domain_expiry;
mod engine;

pub use domain_expiry::normalize_domain;
pub use engine::{check_once, spawn};

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
    "vantage/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/the-last-devops/vantage)"
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

fn down(msg: &str) -> Beat {
    Beat {
        up: false,
        latency_ms: None,
        status_code: None,
        message: Some(truncate(msg, 200)),
        debug: None,
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

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
