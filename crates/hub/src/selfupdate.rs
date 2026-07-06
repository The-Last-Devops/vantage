//! Optional self-update for the `:auto-update` release channel.
//!
//! The hub polls ghcr for the rolling `:auto-update` tag's digest; when it changes
//! (a newer image was pushed) the hub **triggers a rolling redeploy** by patching its
//! own Deployment's pod-template with the new digest annotation. Kubernetes then does a
//! `RollingUpdate` (`maxUnavailable: 0, maxSurge: 1`): it brings up a fresh pod that
//! pulls the new image, waits for it to pass the readiness probe, and only then sends
//! SIGTERM to the old pod — which drains via graceful shutdown. Zero downtime, no 503.
//! (The old design called `std::process::exit(0)`, which restarts the pod *in place*
//! with no surge — a single-replica gap that 503s every update.)
//!
//! It never downloads or executes a binary — the update is a normal image pull, same
//! trust model as a manual redeploy. The patch is idempotent: the annotation value is
//! the digest, so concurrent replicas patch the same value and a re-patch is a no-op.
//!
//! Gated HARD: only when the baked channel is `auto`, running under Kubernetes, and not
//! disabled via `AUTO_UPDATE=0`. Needs a ServiceAccount allowed to `get`/`patch` its own
//! Deployment (the Helm chart wires this up) plus `HUB_DEPLOYMENT_NAME`. The baseline
//! digest is the one present at startup, so a rollout fires only after a genuinely newer
//! push — no crash-loop.

use std::time::Duration;

const REPO: &str = "the-last-devops/vantage-hub";
const TAG: &str = "auto-update";
const POLL: Duration = Duration::from_secs(300);
/// Pod-template annotation we stamp with the target digest to trigger a rollout.
const DIGEST_ANNOTATION: &str = "vantage.dev/auto-update-digest";
/// In-cluster ServiceAccount credentials mount.
const SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

pub fn spawn() {
    if env!("VANTAGE_CHANNEL") != "auto"
        || std::env::var("AUTO_UPDATE").as_deref() == Ok("0")
        || std::env::var("KUBERNETES_SERVICE_HOST").is_err()
    {
        return;
    }
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let baseline = match fetch_digest(&client).await {
            Some(d) => d,
            None => {
                tracing::warn!("self-update: couldn't read initial :auto-update digest — disabled");
                return;
            }
        };
        tracing::info!(%baseline, "self-update: watching ghcr :auto-update");
        let mut tick = tokio::time::interval(POLL);
        tick.tick().await; // consume the immediate first tick
        loop {
            tick.tick().await;
            if let Some(cur) = fetch_digest(&client).await {
                if cur != baseline {
                    if rollout_possible() {
                        tracing::warn!(%baseline, %cur, "newer :auto-update image — rolling the Deployment");
                        if trigger_rollout(&cur).await.is_some() {
                            // k8s will surge a new pod, then SIGTERM us. Stop polling and keep
                            // serving until terminated so the rollout is zero-downtime.
                            break;
                        }
                        // Configured for rollout but the patch failed (transient API error) —
                        // retry on the next tick rather than restarting in place.
                    } else {
                        // No rollout RBAC (raw manifest without HUB_DEPLOYMENT_NAME + the
                        // ServiceAccount). Fall back to the simple in-place restart so
                        // auto-update still works — k8s re-pulls on the pod restart. Brief
                        // downtime at single replica; add the RBAC for zero-downtime.
                        tracing::warn!(%baseline, %cur, "newer :auto-update image — no rollout RBAC; exiting for an in-place restart");
                        std::process::exit(0);
                    }
                }
            }
        }
    });
}

/// Whether we're wired to trigger a rolling redeploy: the Deployment name is known and
/// the in-cluster ServiceAccount token is mounted. When false we fall back to exit(0).
fn rollout_possible() -> bool {
    std::env::var("HUB_DEPLOYMENT_NAME").is_ok_and(|v| !v.trim().is_empty())
        && std::path::Path::new(&format!("{SA_DIR}/token")).exists()
}

/// Patch this hub's own Deployment pod-template with the new digest, which makes k8s
/// roll it out. Uses the in-cluster ServiceAccount token + CA; returns `None` (logged)
/// on any failure so the caller retries. The patch is a no-op when the annotation is
/// already the target digest, so concurrent replicas don't trigger duplicate rollouts.
async fn trigger_rollout(digest: &str) -> Option<()> {
    let host = std::env::var("KUBERNETES_SERVICE_HOST").ok()?;
    let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
    let Ok(deployment) = std::env::var("HUB_DEPLOYMENT_NAME") else {
        tracing::error!(
            "self-update: HUB_DEPLOYMENT_NAME unset — can't roll out (set it in the chart)"
        );
        return None;
    };
    let ws = std::fs::read_to_string(format!("{SA_DIR}/workspace")).ok()?;
    let token = std::fs::read_to_string(format!("{SA_DIR}/token")).ok()?;
    let ca = std::fs::read(format!("{SA_DIR}/ca.crt")).ok()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).ok()?)
        .build()
        .ok()?;
    let url = format!(
        "https://{host}:{port}/apis/apps/v1/workspaces/{}/deployments/{deployment}",
        ws.trim()
    );
    let body = serde_json::json!({
        "spec": { "template": { "metadata": { "annotations": { DIGEST_ANNOTATION: digest } } } }
    });
    let resp = client
        .patch(&url)
        .bearer_auth(token.trim())
        .header("Content-Type", "application/merge-patch+json")
        .json(&body)
        .send()
        .await
        .ok()?;
    if resp.status().is_success() {
        tracing::warn!(%digest, "self-update: patched Deployment — k8s is rolling out the new image");
        Some(())
    } else {
        tracing::error!(
            status = %resp.status(),
            "self-update: Deployment patch failed (check ServiceAccount RBAC: get/patch on this Deployment)"
        );
        None
    }
}

/// Current digest of `ghcr.io/<REPO>:<TAG>` via an anonymous pull token (public).
async fn fetch_digest(client: &reqwest::Client) -> Option<String> {
    let tok: serde_json::Value = client
        .get(format!(
            "https://ghcr.io/token?scope=repository:{REPO}:pull"
        ))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let token = tok.get("token")?.as_str()?;
    let resp = client
        .head(format!("https://ghcr.io/v2/{REPO}/manifests/{TAG}"))
        .bearer_auth(token)
        .header(
            "Accept",
            "application/vnd.oci.image.index.v1+json, \
             application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .send()
        .await
        .ok()?;
    resp.headers()
        .get("docker-content-digest")?
        .to_str()
        .ok()
        .map(String::from)
}
