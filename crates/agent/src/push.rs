//! Pushing reports to the hub: the POST itself, the http→https self-heal on a
//! redirect, and the graceful-shutdown signal.

use serde::Serialize;
use shared::{IngestAck, MetricsReport, API_KEY_HEADER};

/// Resolves on the first Ctrl-C or SIGTERM.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}

/// Outcome of a single report POST.
pub enum Sent {
    /// Accepted; carries the optional next interval (secs) and the hub's build id
    /// (for the auto-update channel) from the IngestAck.
    Ok {
        next: Option<u64>,
        hub_build: Option<String>,
    },
    /// Hub returned a 3xx; carries the Location header (may be empty / relative).
    Redirect(String),
    /// Hub returned a non-success, non-redirect status.
    Rejected(u16),
    /// Transport error (couldn't reach the hub).
    Failed,
}

pub async fn send_report(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    report: &MetricsReport,
) -> Sent {
    post_report(client, url, api_key, report).await
}

/// POST any JSON-serializable report (host `MetricsReport` or `KubeReport`) to the
/// hub and interpret the response the same way. Both ingest paths answer with an
/// `IngestAck`, so the redirect/reject/interval-ramp handling is identical.
pub async fn post_report<T: Serialize>(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    report: &T,
) -> Sent {
    match client
        .post(url)
        .header(API_KEY_HEADER, api_key)
        .json(report)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let ack = resp.json::<IngestAck>().await.ok();
            let next = ack
                .as_ref()
                .map(|a| a.next_interval_secs)
                .filter(|n| *n > 0);
            let hub_build = ack.and_then(|a| a.hub_build);
            Sent::Ok { next, hub_build }
        }
        Ok(resp) if resp.status().is_redirection() => Sent::Redirect(
            resp.headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string(),
        ),
        Ok(resp) => Sent::Rejected(resp.status().as_u16()),
        Err(_) => Sent::Failed,
    }
}

/// True if this agent should self-restart to pick up a newer image. Gated HARD so
/// only the rolling `:auto-update` channel ever does it. All must hold:
///
/// - baked channel is `auto` (stable/dev images can never self-update);
/// - not disabled via `AUTO_UPDATE=0` (kill switch);
/// - running under Kubernetes (else a restart wouldn't re-pull the image);
/// - our build id is known and differs from the hub's advertised build.
///
/// The restart itself is a clean `exit(0)` — k8s does the actual pull (requires
/// `imagePullPolicy: Always` on the rolling tag).
pub fn should_self_update(hub_build: Option<&str>) -> bool {
    if env!("VANTAGE_CHANNEL") != "auto" {
        return false;
    }
    if std::env::var("AUTO_UPDATE").as_deref() == Ok("0") {
        return false;
    }
    if std::env::var("KUBERNETES_SERVICE_HOST").is_err() {
        return false;
    }
    let me = env!("GIT_SHA");
    if me == "unknown" {
        return false;
    }
    matches!(hub_build, Some(h) if !h.is_empty() && h != me)
}

/// Deterministic per-host jitter in `0..max_secs`, so the whole fleet doesn't
/// restart (and re-pull) at the same instant. Derived from the hostname so each
/// host keeps a stable, spread-out offset.
pub fn restart_jitter(hostname: &str, max_secs: u64) -> std::time::Duration {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hostname.hash(&mut h);
    std::time::Duration::from_secs(h.finish() % max_secs.max(1))
}

/// scheme://authority of a URL (everything before the path).
fn origin(url: &str) -> &str {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let end = rest
                .find('/')
                .map(|i| scheme.len() + 3 + i)
                .unwrap_or(url.len());
            &url[..end]
        }
        None => url,
    }
}

/// Decide whether to auto-upgrade to https after a redirect. Returns the https URL
/// to retry on ONLY when `current` is http AND the redirect stays on the SAME host
/// (a relative Location, or one pointing at our own https origin). Returns None
/// otherwise — we must never follow a redirect to a different host, which could
/// leak the agent's enrollment token.
pub fn upgrade_target(current: &str, location: &str) -> Option<String> {
    let rest = current.strip_prefix("http://")?;
    let https_url = format!("https://{rest}");
    let same_host = location.is_empty()
        || location.starts_with('/')
        || location.starts_with(origin(&https_url));
    same_host.then_some(https_url)
}

#[cfg(test)]
mod redirect_tests {
    use super::{origin, upgrade_target};

    #[test]
    fn origin_strips_path() {
        assert_eq!(
            origin("https://h.example.net/pub/ingest"),
            "https://h.example.net"
        );
        assert_eq!(origin("http://localhost:8080/x"), "http://localhost:8080");
        assert_eq!(origin("https://h.example.net"), "https://h.example.net");
    }

    #[test]
    fn upgrades_same_host_http_to_https() {
        let cur = "http://mon.example.net/pub/ingest";
        // Absolute Location to our own https origin → upgrade.
        assert_eq!(
            upgrade_target(cur, "https://mon.example.net/pub/ingest").as_deref(),
            Some("https://mon.example.net/pub/ingest"),
        );
        // Relative Location → same host → upgrade.
        assert_eq!(
            upgrade_target(cur, "/pub/ingest").as_deref(),
            Some("https://mon.example.net/pub/ingest"),
        );
        // Empty Location (some proxies omit it) → assume same host → upgrade.
        assert!(upgrade_target(cur, "").is_some());
    }

    #[test]
    fn never_follows_to_a_different_host() {
        // A redirect to ANOTHER host must NOT be followed (token-leak guard).
        assert_eq!(
            upgrade_target(
                "http://mon.example.net/pub/ingest",
                "https://evil.example.com/x"
            ),
            None
        );
        // Already https → nothing to upgrade.
        assert_eq!(
            upgrade_target("https://mon.example.net/pub/ingest", "/pub/ingest"),
            None
        );
    }
}
