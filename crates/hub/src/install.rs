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

const AGENT_IMAGE: &str = "ghcr.io/the-last-devops/vantage-agent:main";
// The rolling auto-update channel — the agent self-restarts to follow the hub.
const AGENT_IMAGE_AUTO: &str = "ghcr.io/the-last-devops/vantage-agent:auto-update";

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
    /// `1`/`true` → use the rolling `:auto-update` image so the agent self-updates.
    /// Anything else (default) pins the `:main` build (manual updates).
    #[serde(default)]
    autoupdate: Option<String>,
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
    let auto = matches!(p.autoupdate.as_deref(), Some("1") | Some("true"));
    let (image, auto_env) = if auto {
        (AGENT_IMAGE_AUTO, "1")
    } else {
        (AGENT_IMAGE, "0")
    };
    let body = AGENT_MANIFEST
        .replace("<HUB_URL>", &base_url(&headers))
        .replace("<API_KEY>", &p.key)
        .replace("<CLUSTER>", cluster)
        .replace("<NAMESPACE>", ns)
        .replace("<IMAGE>", image)
        .replace("<AUTO_UPDATE>", auto_env);
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
