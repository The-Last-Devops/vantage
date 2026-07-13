//! Self-service install assets served by the hub itself, so the snippets shown
//! in "Add system" are fully self-contained against the hub's own domain — no
//! GHCR chart publish or `git clone` needed.
//!
//!   GET /k8s/agent.yaml?key=&cluster=&ns=   → ready-to-apply manifest: the per-node
//!                                             DaemonSet + the one-per-cluster collector
//!   GET /install.sh                          → native-binary installer (curl | sh)
//!
//! Both are public (kubectl / curl fetch them without a session); they only echo
//! values the caller already supplied — they expose no hub secrets.

use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderMap},
    response::Response,
};
use serde::Deserialize;

const AGENT_IMAGE_REPO: &str = "ghcr.io/the-last-devops/vantage-agent";
/// Image tag used when the caller doesn't pick one.
const DEFAULT_AGENT_TAG: &str = "latest";

/// Accept only a safe image tag (it's substituted straight into the served YAML).
/// Docker tags are `[A-Za-z0-9_][A-Za-z0-9._-]{0,127}` — reject anything else so a
/// crafted `?tag=` can't inject manifest content. Empty/invalid → the default tag.
fn sanitize_tag(tag: Option<&str>) -> &str {
    match tag {
        Some(t)
            if !t.is_empty()
                && t.len() <= 128
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) =>
        {
            t
        }
        _ => DEFAULT_AGENT_TAG,
    }
}

/// Reconstruct the hub's public base URL from the request the caller hit, so the
/// agent reports back to the same domain (works behind an ingress / TLS).
fn base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:8080");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_else(|| {
            if host.starts_with("localhost") {
                "http"
            } else {
                "https"
            }
        });
    format!("{proto}://{host}")
}

#[derive(Deserialize)]
pub struct AgentParams {
    #[serde(default)]
    key: String,
    #[serde(default)]
    cluster: String,
    /// k8s namespace to install the DaemonSet into (NOT the RBAC workspace, which
    /// the API key already encodes). Defaults to `vantage`.
    ns: Option<String>,
    /// Image tag to run (e.g. `latest`, `main`, `3.0.0`). Sanitized; anything
    /// invalid falls back to the default. Updates are driven by re-applying with a
    /// new tag (or `imagePullPolicy: Always` on a moving tag) — no in-agent updater.
    #[serde(default)]
    tag: Option<String>,
}

const AGENT_MANIFEST: &str = include_str!("../templates/agent.yaml.tmpl");

/// GET /k8s/agent.yaml — render the DaemonSet manifest with the caller's values.
pub async fn k8s_agent_yaml(headers: HeaderMap, Query(p): Query<AgentParams>) -> Response {
    let cluster = if p.cluster.is_empty() {
        "my-cluster"
    } else {
        &p.cluster
    };
    let ns =
        p.ns.as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("vantage");
    let image = format!("{AGENT_IMAGE_REPO}:{}", sanitize_tag(p.tag.as_deref()));
    let body = AGENT_MANIFEST
        .replace("<HUB_URL>", &base_url(&headers))
        .replace("<API_KEY>", &p.key)
        .replace("<CLUSTER>", cluster)
        .replace("<NAMESPACE>", ns)
        .replace("<IMAGE>", &image);
    Response::builder()
        .header(header::CONTENT_TYPE, "application/yaml")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap()
}

const INSTALL_SH: &str = include_str!("../templates/install.sh");

/// GET /install.sh — native-binary installer; reads HUB_URL/API_KEY from the env
/// the caller pipes in (`curl … | HUB_URL=… API_KEY=… sh`).
pub async fn install_sh() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/x-shellscript")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(INSTALL_SH))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tag_when_absent_or_empty() {
        assert_eq!(sanitize_tag(None), DEFAULT_AGENT_TAG);
        assert_eq!(sanitize_tag(Some("")), DEFAULT_AGENT_TAG);
    }

    #[test]
    fn accepts_valid_tags() {
        assert_eq!(sanitize_tag(Some("latest")), "latest");
        assert_eq!(sanitize_tag(Some("main")), "main");
        assert_eq!(sanitize_tag(Some("3.0.0")), "3.0.0");
        assert_eq!(sanitize_tag(Some("v3.0.0-rc.1")), "v3.0.0-rc.1");
    }

    #[test]
    fn rejects_injection_and_out_of_range() {
        // Anything that could break out of the image ref / inject YAML falls back.
        for bad in ["a b", "a/b", "a:b", "a\nb", "a\"b", "тест", "a b"] {
            assert_eq!(sanitize_tag(Some(bad)), DEFAULT_AGENT_TAG, "tag {bad:?}");
        }
        let too_long = "a".repeat(129);
        assert_eq!(sanitize_tag(Some(&too_long)), DEFAULT_AGENT_TAG);
    }
}
