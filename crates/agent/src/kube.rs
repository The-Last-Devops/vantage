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

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use shared::{KubeContainerStat, KubeDeploymentStat, KubeNamespaceStat, KubeReport};

use crate::push::{post_report, shutdown_signal, upgrade_target, Sent};
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
                    Sent::Ok { next } => {
                        if let Some(secs) = next {
                            interval = Duration::from_secs(secs);
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
    // ReplicaSets let us walk pod → RS → Deployment for the owning workload. Soft:
    // if the ServiceAccount lacks `replicasets` RBAC (e.g. agent upgraded before the
    // ClusterRole), degrade to showing the ReplicaSet as the workload rather than
    // failing the whole snapshot.
    let rs_items: Vec<ReplicaSetItem> =
        list_soft(client, api_base, token, "/apis/apps/v1/replicasets").await;
    // Per-container CPU/memory from metrics-server. Optional: many clusters don't
    // run it, so treat any failure (404 / not installed) as "no usage" rather than
    // failing the whole snapshot — the metadata is still worth reporting.
    let metric_items: Vec<PodMetricsItem> =
        list_soft(client, api_base, token, "/apis/metrics.k8s.io/v1beta1/pods").await;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut report = aggregate(
        ns_items,
        pod_items,
        deploy_items,
        rs_items,
        metric_items,
        cluster,
        ts,
    );
    report.k8s_version = fetch_version(client, api_base, token).await;
    Ok(report)
}

/// GET the apiserver `/version` → its `gitVersion` (e.g. "v1.29.4"). Soft: returns
/// "" on any failure, so a version probe never fails the snapshot.
async fn fetch_version(client: &reqwest::Client, api_base: &str, token: &str) -> String {
    #[derive(Deserialize, Default)]
    struct Ver {
        #[serde(default, rename = "gitVersion")]
        git_version: String,
    }
    let url = format!("{api_base}/version");
    match client.get(&url).bearer_auth(token).send().await {
        Ok(r) if r.status().is_success() => r
            .json::<Ver>()
            .await
            .map(|v| v.git_version)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Fold raw apiserver items into a `KubeReport`. Pure (no I/O) so it's unit-testable.
#[allow(clippy::too_many_arguments)]
fn aggregate(
    ns_items: Vec<NsItem>,
    pod_items: Vec<PodItem>,
    deploy_items: Vec<DeployItem>,
    rs_items: Vec<ReplicaSetItem>,
    metric_items: Vec<PodMetricsItem>,
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

    for p in &pod_items {
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

    let containers = build_containers(&pod_items, &rs_items, &metric_items);

    KubeReport {
        ts,
        cluster: cluster.to_string(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        k8s_version: String::new(), // set by collect() from the apiserver /version
        namespaces: ns.into_values().collect(),
        deployments,
        containers,
    }
}

/// Fold pods + ReplicaSets + metrics-server usage into per-container rows, each
/// carrying its pod's metadata (node, phase, labels) and resolved owning workload
/// (pod → ReplicaSet → Deployment). Pure (no I/O) so it's unit-testable.
fn build_containers(
    pods: &[PodItem],
    rs_items: &[ReplicaSetItem],
    metrics: &[PodMetricsItem],
) -> Vec<KubeContainerStat> {
    // ReplicaSet -> owning workload (usually a Deployment), keyed by (namespace, rs).
    let mut rs_workload: HashMap<(String, String), (String, String)> = HashMap::new();
    for rs in rs_items {
        let wl = controller_ref(&rs.metadata.owner_references)
            .map(|o| (o.kind.clone(), o.name.clone()))
            .unwrap_or_else(|| ("ReplicaSet".to_string(), rs.metadata.name.clone()));
        rs_workload.insert(
            (rs.metadata.namespace.clone(), rs.metadata.name.clone()),
            wl,
        );
    }

    // (namespace, pod, container) -> (cpu_millicores, mem_bytes) from metrics-server.
    let mut usage: HashMap<(String, String, String), (u64, u64)> = HashMap::new();
    for m in metrics {
        for c in &m.containers {
            usage.insert(
                (
                    m.metadata.namespace.clone(),
                    m.metadata.name.clone(),
                    c.name.clone(),
                ),
                (
                    parse_cpu_millicores(&c.usage.cpu),
                    parse_mem_bytes(&c.usage.memory),
                ),
            );
        }
    }

    let mut out = Vec::new();
    for p in pods {
        let ns = &p.metadata.namespace;
        let pod = &p.metadata.name;
        // Resolve the top-level workload: pod's controller, hopping RS -> Deployment.
        let (workload_kind, workload) = match controller_ref(&p.metadata.owner_references) {
            Some(o) if o.kind == "ReplicaSet" => rs_workload
                .get(&(ns.clone(), o.name.clone()))
                .cloned()
                .unwrap_or_else(|| ("ReplicaSet".to_string(), o.name.clone())),
            Some(o) => (o.kind.clone(), o.name.clone()),
            None => (String::new(), String::new()),
        };
        for cs in &p.status.container_statuses {
            let (cpu_millicores, mem_bytes) = usage
                .get(&(ns.clone(), pod.clone(), cs.name.clone()))
                .copied()
                .unwrap_or((0, 0));
            out.push(KubeContainerStat {
                namespace: ns.clone(),
                pod: pod.clone(),
                container: cs.name.clone(),
                node: p.spec.node_name.clone(),
                phase: p.status.phase.clone(),
                workload: workload.clone(),
                workload_kind: workload_kind.clone(),
                cpu_millicores,
                mem_bytes,
                restarts: cs.restart_count,
                labels: p.metadata.labels.clone(),
            });
        }
    }
    out.sort_by(|a, b| {
        a.namespace
            .cmp(&b.namespace)
            .then(a.pod.cmp(&b.pod))
            .then(a.container.cmp(&b.container))
    });
    out
}

/// The controlling ownerReference (`controller: true`), else the first one
/// (older objects don't always set the flag). None for a bare pod/RS.
fn controller_ref(refs: &[OwnerRef]) -> Option<&OwnerRef> {
    refs.iter()
        .find(|o| o.controller == Some(true))
        .or_else(|| refs.first())
}

/// Parse a Kubernetes CPU quantity into **millicores**. Accepts nanocores (`n`),
/// microcores (`u`), millicores (`m`), or whole cores (no suffix).
fn parse_cpu_millicores(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let (num, per_milli) = if let Some(v) = s.strip_suffix('n') {
        (v, 1e6) // nanocores  -> /1e6 = millicores
    } else if let Some(v) = s.strip_suffix('u') {
        (v, 1e3) // microcores -> /1e3
    } else if let Some(v) = s.strip_suffix('m') {
        (v, 1.0) // already millicores
    } else {
        (s, 0.001) // cores -> /0.001 = *1000
    };
    (num.trim().parse::<f64>().unwrap_or(0.0) / per_milli).round() as u64
}

/// Parse a Kubernetes memory quantity into **bytes**. Accepts binary suffixes
/// (Ki/Mi/Gi/Ti/Pi/Ei), decimal SI (k/M/G/T/P/E), or a plain byte count.
fn parse_mem_bytes(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    // Two-letter binary suffixes first so "Mi" isn't shadowed by "M".
    const SUFFIXES: &[(&str, f64)] = &[
        ("Ki", 1024.0),
        ("Mi", 1_048_576.0),
        ("Gi", 1_073_741_824.0),
        ("Ti", 1_099_511_627_776.0),
        ("Pi", 1_125_899_906_842_624.0),
        ("Ei", 1_152_921_504_606_846_976.0),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ];
    for (suf, mult) in SUFFIXES {
        if let Some(v) = s.strip_suffix(suf) {
            return (v.trim().parse::<f64>().unwrap_or(0.0) * mult).round() as u64;
        }
    }
    s.parse::<u64>()
        .unwrap_or_else(|_| s.parse::<f64>().unwrap_or(0.0).round() as u64)
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

/// Single-page GET for an OPTIONAL list API, returning an empty Vec (never an
/// error) when it's unavailable — e.g. `metrics.k8s.io` 404s on clusters without
/// metrics-server. No pagination: the metrics API doesn't chunk, and one missing
/// add-on must not fail the whole snapshot.
async fn list_soft<T: DeserializeOwned>(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    path: &str,
) -> Vec<T> {
    let url = format!("{api_base}{path}");
    let resp = match client.get(&url).bearer_auth(token).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, path, "optional kube API request failed — skipping");
            return Vec::new();
        }
    };
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), path, "optional kube API unavailable — skipping");
        return Vec::new();
    }
    match resp.json::<List<T>>().await {
        Ok(page) => page.items,
        Err(e) => {
            tracing::debug!(error = %e, path, "decode of optional kube API failed — skipping");
            Vec::new()
        }
    }
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
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default, rename = "ownerReferences")]
    owner_references: Vec<OwnerRef>,
}

#[derive(Deserialize, Default, Clone)]
struct OwnerRef {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    name: String,
    /// Kubernetes marks the managing controller with `controller: true`.
    #[serde(default)]
    controller: Option<bool>,
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
    spec: PodSpec,
    #[serde(default)]
    status: PodStatus,
}

#[derive(Deserialize, Default)]
struct PodSpec {
    #[serde(default, rename = "nodeName")]
    node_name: String,
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
    #[serde(default)]
    name: String,
    #[serde(default, rename = "restartCount")]
    restart_count: u32,
}

#[derive(Deserialize)]
struct ReplicaSetItem {
    #[serde(default)]
    metadata: Meta,
}

// --- metrics.k8s.io (metrics-server) shapes ---

#[derive(Deserialize)]
struct PodMetricsItem {
    #[serde(default)]
    metadata: Meta,
    #[serde(default)]
    containers: Vec<ContainerMetrics>,
}

#[derive(Deserialize, Default)]
struct ContainerMetrics {
    #[serde(default)]
    name: String,
    #[serde(default)]
    usage: Usage,
}

#[derive(Deserialize, Default)]
struct Usage {
    /// CPU quantity, e.g. "250m", "1", "500000000n".
    #[serde(default)]
    cpu: String,
    /// Memory quantity, e.g. "134217728", "128Mi".
    #[serde(default)]
    memory: String,
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
            Vec::new(),
            Vec::new(),
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
    fn parses_cpu_quantities_to_millicores() {
        assert_eq!(parse_cpu_millicores("250m"), 250);
        assert_eq!(parse_cpu_millicores("1"), 1000); // 1 core
        assert_eq!(parse_cpu_millicores("2"), 2000);
        assert_eq!(parse_cpu_millicores("500000000n"), 500); // 5e8 nanocores = 500m
        assert_eq!(parse_cpu_millicores("1500u"), 2); // 1500 microcores ≈ 1.5m -> 2
        assert_eq!(parse_cpu_millicores(""), 0);
        assert_eq!(parse_cpu_millicores("garbage"), 0);
    }

    #[test]
    fn parses_memory_quantities_to_bytes() {
        assert_eq!(parse_mem_bytes("128Mi"), 128 * 1024 * 1024);
        assert_eq!(parse_mem_bytes("1Gi"), 1024 * 1024 * 1024);
        assert_eq!(parse_mem_bytes("512Ki"), 512 * 1024);
        assert_eq!(parse_mem_bytes("134217728"), 134_217_728); // plain bytes
        assert_eq!(parse_mem_bytes("1M"), 1_000_000); // decimal SI
        assert_eq!(parse_mem_bytes(""), 0);
    }

    #[test]
    fn build_containers_resolves_workload_and_joins_usage() {
        // web pod -> ReplicaSet web-abc -> Deployment web; a bare DaemonSet pod;
        // metrics-server usage present for web's container only.
        let pods: Vec<PodItem> = items(
            r#"{"items":[
            {"metadata":{"name":"web-abc-1","namespace":"default","labels":{"app":"web"},
                "ownerReferences":[{"kind":"ReplicaSet","name":"web-abc","controller":true}]},
             "spec":{"nodeName":"node-1"},
             "status":{"phase":"Running","containerStatuses":[
                {"name":"app","restartCount":2},{"name":"sidecar","restartCount":0}]}},
            {"metadata":{"name":"log-1","namespace":"kube-system","labels":{"app":"logger"},
                "ownerReferences":[{"kind":"DaemonSet","name":"logger","controller":true}]},
             "spec":{"nodeName":"node-2"},
             "status":{"phase":"Running","containerStatuses":[{"name":"log","restartCount":0}]}}
        ]}"#,
        );
        let rs: Vec<ReplicaSetItem> = items(
            r#"{"items":[
            {"metadata":{"name":"web-abc","namespace":"default",
                "ownerReferences":[{"kind":"Deployment","name":"web","controller":true}]}}
        ]}"#,
        );
        let metrics: Vec<PodMetricsItem> = items(
            r#"{"items":[
            {"metadata":{"name":"web-abc-1","namespace":"default","containers":[]},
             "containers":[
                {"name":"app","usage":{"cpu":"250m","memory":"128Mi"}},
                {"name":"sidecar","usage":{"cpu":"10m","memory":"16Mi"}}]}
        ]}"#,
        );
        let out = build_containers(&pods, &rs, &metrics);
        // Sorted by (namespace, pod, container): default/web-abc-1/{app,sidecar}, then kube-system/log-1/log.
        assert_eq!(out.len(), 3);

        let app = &out[0];
        assert_eq!(app.container, "app");
        assert_eq!(app.workload_kind, "Deployment"); // hopped RS -> Deployment
        assert_eq!(app.workload, "web");
        assert_eq!(app.node, "node-1");
        assert_eq!(app.cpu_millicores, 250);
        assert_eq!(app.mem_bytes, 128 * 1024 * 1024);
        assert_eq!(app.restarts, 2);
        assert_eq!(app.labels.get("app").map(String::as_str), Some("web"));

        let side = &out[1];
        assert_eq!(side.container, "sidecar");
        assert_eq!(side.cpu_millicores, 10);

        let log = &out[2];
        assert_eq!(log.workload_kind, "DaemonSet");
        assert_eq!(log.workload, "logger");
        assert_eq!(log.cpu_millicores, 0); // no metrics for this pod -> 0
        assert_eq!(log.mem_bytes, 0);
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
