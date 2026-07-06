//! Cluster-scoped Kubernetes collector (`AGENT_KIND=k8s-cluster`).
//!
//! Unlike the per-node metrics loop in `main.rs`, exactly ONE of these runs per
//! cluster (a Deployment, not the DaemonSet). It reads the in-cluster
//! ServiceAccount token + CA that Kubernetes mounts into the pod, queries the
//! kube-apiserver's REST API directly with `reqwest` (deliberately NOT the heavy
//! `kube-rs` client — see the "stay lightweight" principle), tallies namespaces /
//! deployments / pods, and pushes a `KubeReport` to `POST /pub/kube`.
//!
//! It only ever GETs — the ServiceAccount is granted `get/list/watch` and nothing
//! more (see the RBAC in the install manifest). No cluster mutation, ever.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use shared::{KubeDeploymentStat, KubeNamespaceStat, KubeReport};

use crate::push::{self, post_report, shutdown_signal, upgrade_target, Sent};
use crate::Config;

/// Where kubelet mounts the pod's ServiceAccount credentials.
const SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

/// Run the cluster collector loop until a shutdown signal arrives. Returns Err only
/// on unrecoverable setup problems (no in-cluster credentials, bad CA); transient
/// per-cycle failures are logged and retried.
pub async fn run(cfg: &Config) -> Result<()> {
    // apiserver address is injected into every pod by kubelet.
    let host = std::env::var("KUBERNETES_SERVICE_HOST").context(
        "KUBERNETES_SERVICE_HOST unset — the k8s-cluster agent must run inside a Kubernetes pod",
    )?;
    let port = std::env::var("KUBERNETES_SERVICE_PORT_HTTPS")
        .or_else(|_| std::env::var("KUBERNETES_SERVICE_PORT"))
        .unwrap_or_else(|_| "443".into());
    // Bracket IPv6 literals so the URL is well-formed.
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let api_base = format!("https://{authority}");

    // Trust the cluster CA (the apiserver cert is cluster-signed, not from a public
    // root). We ADD it to reqwest's built-in webpki roots rather than replacing them.
    let ca_pem = std::fs::read(format!("{SA_DIR}/ca.crt"))
        .with_context(|| format!("reading cluster CA at {SA_DIR}/ca.crt"))?;
    let ca = reqwest::Certificate::from_pem(&ca_pem).context("parsing cluster CA cert")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(ca)
        .user_agent(concat!("vantage-agent/", env!("CARGO_PKG_VERSION")))
        .build()?;

    let cluster = if cfg.cluster.is_empty() {
        "default".to_string()
    } else {
        cfg.cluster.clone()
    };
    let mut kube_url = format!("{}/pub/kube", cfg.hub_url);
    let mut interval = cfg.interval;
    tracing::info!(%api_base, %cluster, ?interval, "k8s-cluster collector started");

    let mut shutdown = Box::pin(shutdown_signal());
    loop {
        // Re-read the token every cycle: projected ServiceAccount tokens rotate
        // (~hourly), so a cached one would eventually 401.
        let token = match std::fs::read_to_string(format!("{SA_DIR}/token")) {
            Ok(t) => t.trim().to_string(),
            Err(e) => {
                tracing::warn!(error = %e, "cannot read ServiceAccount token — retrying next cycle");
                if wait(&mut interval, &mut shutdown).await {
                    break;
                }
                continue;
            }
        };

        match collect(&client, &api_base, &token, &cluster).await {
            Ok(report) => {
                tracing::debug!(
                    namespaces = report.namespaces.len(),
                    deployments = report.deployments.len(),
                    "kube snapshot collected"
                );
                let mut outcome = post_report(&client, &kube_url, &cfg.api_key, &report).await;
                if let Sent::Redirect(ref loc) = outcome {
                    if let Some(https) = upgrade_target(&kube_url, loc) {
                        tracing::warn!("hub redirected http→https — upgrading and retrying");
                        kube_url = https;
                        outcome = post_report(&client, &kube_url, &cfg.api_key, &report).await;
                    }
                }
                match outcome {
                    Sent::Ok { next, hub_build } => {
                        if let Some(secs) = next {
                            interval = Duration::from_secs(secs);
                        }
                        if push::should_self_update(hub_build.as_deref()) {
                            let jit = push::restart_jitter(&cluster, 300);
                            tracing::warn!(
                                jitter_secs = jit.as_secs(),
                                "newer hub build — will restart for auto-update"
                            );
                            tokio::select! {
                                _ = tokio::time::sleep(jit) => {}
                                _ = &mut shutdown => {}
                            }
                            std::process::exit(0);
                        }
                    }
                    Sent::Redirect(loc) => {
                        tracing::warn!(location = %loc, "hub redirected — set HUB_URL to the final URL")
                    }
                    Sent::Rejected(status) => tracing::warn!(%status, "hub rejected kube report"),
                    Sent::Failed => tracing::warn!("failed to reach hub"),
                }
            }
            Err(e) => tracing::warn!(error = %e, "kube snapshot failed — retrying next cycle"),
        }

        if wait(&mut interval, &mut shutdown).await {
            tracing::info!("shutdown signal received — k8s-cluster collector stopping");
            break;
        }
    }
    Ok(())
}

/// Sleep for `interval` or until shutdown; returns true if shutdown fired.
async fn wait(
    interval: &mut Duration,
    shutdown: &mut std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(*interval) => false,
        _ = shutdown => true,
    }
}

/// Query the apiserver and build a single `KubeReport`.
async fn collect(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    cluster: &str,
) -> Result<KubeReport> {
    let ns_items: Vec<NsItem> = list_all(client, api_base, token, "/api/v1/namespaces").await?;
    let pod_items: Vec<PodItem> = list_all(client, api_base, token, "/api/v1/pods").await?;
    let deploy_items: Vec<DeployItem> =
        list_all(client, api_base, token, "/apis/apps/v1/deployments").await?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(aggregate(ns_items, pod_items, deploy_items, cluster, ts))
}

/// Fold raw apiserver items into a `KubeReport`. Pure (no I/O) so it's unit-testable.
fn aggregate(
    ns_items: Vec<NsItem>,
    pod_items: Vec<PodItem>,
    deploy_items: Vec<DeployItem>,
    cluster: &str,
    ts: i64,
) -> KubeReport {
    // Seed a stat row per namespace so empty namespaces still show up.
    let mut ns: BTreeMap<String, KubeNamespaceStat> = BTreeMap::new();
    for n in ns_items {
        ns.insert(
            n.metadata.name.clone(),
            KubeNamespaceStat {
                name: n.metadata.name,
                phase: n.status.phase,
                pods_total: 0,
                pods_running: 0,
                pods_pending: 0,
                pods_failed: 0,
                pods_succeeded: 0,
                restarts: 0,
            },
        );
    }

    for p in pod_items {
        let entry = ns
            .entry(p.metadata.namespace.clone())
            .or_insert_with(|| KubeNamespaceStat {
                name: p.metadata.namespace.clone(),
                phase: String::new(),
                pods_total: 0,
                pods_running: 0,
                pods_pending: 0,
                pods_failed: 0,
                pods_succeeded: 0,
                restarts: 0,
            });
        entry.pods_total += 1;
        match p.status.phase.as_str() {
            "Running" => entry.pods_running += 1,
            "Pending" => entry.pods_pending += 1,
            "Failed" => entry.pods_failed += 1,
            "Succeeded" => entry.pods_succeeded += 1,
            _ => {}
        }
        entry.restarts += p
            .status
            .container_statuses
            .iter()
            .map(|c| c.restart_count)
            .sum::<u32>();
    }

    let mut deployments: Vec<KubeDeploymentStat> = deploy_items
        .into_iter()
        .map(|d| KubeDeploymentStat {
            namespace: d.metadata.namespace,
            name: d.metadata.name,
            desired: d.spec.replicas,
            ready: d.status.ready_replicas,
            available: d.status.available_replicas,
            updated: d.status.updated_replicas,
        })
        .collect();
    deployments.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));

    KubeReport {
        ts,
        cluster: cluster.to_string(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        namespaces: ns.into_values().collect(),
        deployments,
    }
}

/// GET a list endpoint, following the `continue` token so large clusters aren't
/// truncated. Requests pages of 500 and unknown JSON fields are ignored.
async fn list_all<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    path: &str,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    let mut cont = String::new();
    loop {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = if cont.is_empty() {
            format!("{api_base}{path}{sep}limit=500")
        } else {
            format!("{api_base}{path}{sep}limit=500&continue={cont}")
        };
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("GET {path} -> {}", resp.status());
        }
        let page: List<T> = resp
            .json()
            .await
            .with_context(|| format!("decode {path}"))?;
        out.extend(page.items);
        match page.metadata.cont {
            Some(c) if !c.is_empty() => cont = c,
            _ => break,
        }
    }
    Ok(out)
}

// --- minimal apiserver JSON shapes (only the fields we tally) ---

#[derive(Deserialize)]
struct List<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    metadata: ListMeta,
}

#[derive(Deserialize, Default)]
struct ListMeta {
    #[serde(rename = "continue")]
    cont: Option<String>,
}

#[derive(Deserialize, Default)]
struct Meta {
    #[serde(default)]
    name: String,
    #[serde(default)]
    namespace: String,
}

#[derive(Deserialize)]
struct NsItem {
    #[serde(default)]
    metadata: Meta,
    #[serde(default)]
    status: NsStatus,
}

#[derive(Deserialize, Default)]
struct NsStatus {
    #[serde(default)]
    phase: String,
}

#[derive(Deserialize)]
struct PodItem {
    #[serde(default)]
    metadata: Meta,
    #[serde(default)]
    status: PodStatus,
}

#[derive(Deserialize, Default)]
struct PodStatus {
    #[serde(default)]
    phase: String,
    #[serde(default, rename = "containerStatuses")]
    container_statuses: Vec<ContainerStatus>,
}

#[derive(Deserialize, Default)]
struct ContainerStatus {
    #[serde(default, rename = "restartCount")]
    restart_count: u32,
}

#[derive(Deserialize)]
struct DeployItem {
    #[serde(default)]
    metadata: Meta,
    #[serde(default)]
    spec: DeploySpec,
    #[serde(default)]
    status: DeployStatus,
}

#[derive(Deserialize, Default)]
struct DeploySpec {
    #[serde(default)]
    replicas: u32,
}

#[derive(Deserialize, Default)]
struct DeployStatus {
    #[serde(default, rename = "readyReplicas")]
    ready_replicas: u32,
    #[serde(default, rename = "availableReplicas")]
    available_replicas: u32,
    #[serde(default, rename = "updatedReplicas")]
    updated_replicas: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed-down but real-shaped apiserver responses.
    const NS_JSON: &str = r#"{"items":[
        {"metadata":{"name":"default"},"status":{"phase":"Active"}},
        {"metadata":{"name":"kube-system"},"status":{"phase":"Active"}},
        {"metadata":{"name":"old"},"status":{"phase":"Terminating"}}
    ]}"#;
    const POD_JSON: &str = r#"{"items":[
        {"metadata":{"namespace":"default"},"status":{"phase":"Running",
            "containerStatuses":[{"restartCount":2},{"restartCount":1}]}},
        {"metadata":{"namespace":"default"},"status":{"phase":"Pending","containerStatuses":[]}},
        {"metadata":{"namespace":"kube-system"},"status":{"phase":"Running",
            "containerStatuses":[{"restartCount":0}]}},
        {"metadata":{"namespace":"kube-system"},"status":{"phase":"Failed"}}
    ]}"#;
    const DEPLOY_JSON: &str = r#"{"items":[
        {"metadata":{"name":"web","namespace":"default"},"spec":{"replicas":3},
            "status":{"readyReplicas":2,"availableReplicas":2,"updatedReplicas":3}},
        {"metadata":{"name":"api","namespace":"default"},"spec":{"replicas":1},
            "status":{"readyReplicas":1,"availableReplicas":1,"updatedReplicas":1}}
    ]}"#;

    fn items<T: DeserializeOwned>(json: &str) -> Vec<T> {
        serde_json::from_str::<List<T>>(json).unwrap().items
    }

    #[test]
    fn aggregates_pods_deployments_and_namespaces() {
        let rep = aggregate(
            items::<NsItem>(NS_JSON),
            items::<PodItem>(POD_JSON),
            items::<DeployItem>(DEPLOY_JSON),
            "prod",
            1234,
        );
        assert_eq!(rep.cluster, "prod");
        assert_eq!(rep.ts, 1234);

        // Namespaces sorted by name; empty ones (kube-system has pods, "old" empty) kept.
        let names: Vec<_> = rep.namespaces.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["default", "kube-system", "old"]);

        let def = &rep.namespaces[0];
        assert_eq!(def.pods_total, 2);
        assert_eq!(def.pods_running, 1);
        assert_eq!(def.pods_pending, 1);
        assert_eq!(def.restarts, 3); // 2 + 1

        let sys = &rep.namespaces[1];
        assert_eq!(sys.pods_failed, 1);
        assert_eq!(sys.pods_running, 1);
        assert_eq!(sys.restarts, 0);

        let old = &rep.namespaces[2];
        assert_eq!(old.phase, "Terminating");
        assert_eq!(old.pods_total, 0);

        // Deployments sorted by (namespace, name) -> api before web.
        assert_eq!(rep.deployments.len(), 2);
        assert_eq!(rep.deployments[0].name, "api");
        assert_eq!(rep.deployments[1].name, "web");
        assert_eq!(rep.deployments[1].desired, 3);
        assert_eq!(rep.deployments[1].ready, 2);
    }

    #[test]
    fn list_meta_continue_token_parses() {
        // The pagination cursor the apiserver returns on a truncated page.
        let page: List<NsItem> =
            serde_json::from_str(r#"{"items":[],"metadata":{"continue":"abc123"}}"#).unwrap();
        assert_eq!(page.metadata.cont.as_deref(), Some("abc123"));
        // Absent metadata / continue -> None (loop terminates).
        let last: List<NsItem> = serde_json::from_str(r#"{"items":[]}"#).unwrap();
        assert_eq!(last.metadata.cont, None);
    }
}
