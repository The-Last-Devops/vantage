//! Types shared between the agent and the hub.

pub mod tunnel;

use serde::{Deserialize, Serialize};

/// A single metrics report pushed by an agent to the hub.
///
/// The agent authenticates with an enrollment token (sent via header),
/// so the body itself carries only the measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsReport {
    /// Unix epoch seconds when the sample was taken on the agent.
    pub ts: i64,
    /// Hostname reported by the agent (informational).
    pub hostname: String,
    /// Overall CPU usage, 0.0 - 100.0.
    pub cpu_percent: f32,
    /// Used memory in bytes.
    pub mem_used: u64,
    /// Total memory in bytes.
    pub mem_total: u64,
    /// Used swap in bytes.
    pub swap_used: u64,
    /// Total swap in bytes.
    pub swap_total: u64,
    /// Available memory in bytes — Linux MemAvailable (what apps can allocate without
    /// swapping, i.e. free + reclaimable cache). 0 where unavailable. `mem_used` already
    /// excludes reclaimable cache, so `mem_used + mem_available ≈ mem_total`.
    #[serde(default)]
    pub mem_available: u64,
    /// Buffer cache in bytes (Linux /proc/meminfo `Buffers`). 0 elsewhere.
    #[serde(default)]
    pub mem_buffers: u64,
    /// Page cache in bytes (Linux `Cached` + `SReclaimable`). 0 elsewhere.
    #[serde(default)]
    pub mem_cached: u64,
    /// Free (untouched) memory in bytes (Linux `MemFree`). 0 elsewhere.
    #[serde(default)]
    pub mem_free: u64,
    /// Aggregate used disk in bytes across mounted filesystems.
    pub disk_used: u64,
    /// Aggregate total disk in bytes across mounted filesystems.
    pub disk_total: u64,
    /// Bytes received since boot (cumulative).
    pub net_rx: u64,
    /// Bytes transmitted since boot (cumulative).
    pub net_tx: u64,
    /// Load average over the last minute.
    pub load1: f64,
    /// Load average over the last 5 minutes.
    #[serde(default)]
    pub load5: f64,
    /// Load average over the last 15 minutes.
    #[serde(default)]
    pub load15: f64,
    /// CPU time breakdown as percentages of total (from /proc/stat on Linux;
    /// all 0 where unavailable, e.g. macOS — then only `cpu_percent` is shown).
    #[serde(default)]
    pub cpu_user: f32,
    #[serde(default)]
    pub cpu_system: f32,
    #[serde(default)]
    pub cpu_iowait: f32,
    #[serde(default)]
    pub cpu_steal: f32,
    /// Seconds since the machine booted.
    pub uptime: u64,

    // --- system classification (lets the hub group node / docker / k8s) ---
    /// System kind: "node" (plain host), "docker" (host + containers), or "k8s"
    /// (a Kubernetes node). Empty is treated as "node".
    #[serde(default)]
    pub kind: String,
    /// For k8s nodes, the cluster name they belong to (groups nodes into a
    /// cluster in the UI). Empty for node/docker.
    #[serde(default)]
    pub cluster: String,

    // --- host metadata (mostly static; the hub stores the latest on the server row) ---
    /// Agent binary version (CARGO_PKG_VERSION).
    #[serde(default)]
    pub agent_version: String,
    /// OS kernel version, e.g. "6.5.0-generic".
    #[serde(default)]
    pub kernel: String,
    /// CPU model/brand string.
    #[serde(default)]
    pub cpu_model: String,
    /// Number of logical CPU cores.
    #[serde(default)]
    pub cpu_cores: u32,

    /// Disk bytes read since boot (cumulative; 0 if unavailable).
    #[serde(default)]
    pub disk_read: u64,
    /// Disk bytes written since boot (cumulative; 0 if unavailable).
    #[serde(default)]
    pub disk_write: u64,
    /// Disk I/O utilization: % of the interval the busiest disk was servicing I/O
    /// (iostat-style %util, 0–100). 0 if unavailable (non-Linux).
    #[serde(default)]
    pub disk_util: f32,

    /// Temperature sensors (sensor label -> degrees Celsius).
    #[serde(default)]
    pub temps: Vec<TempReading>,
    /// Per-container stats (empty if Docker isn't available).
    #[serde(default)]
    pub containers: Vec<ContainerStat>,
    /// Per-GPU stats (empty if no GPU / nvidia-smi unavailable).
    #[serde(default)]
    pub gpus: Vec<GpuStat>,
}

/// One GPU's usage at sample time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuStat {
    pub name: String,
    /// Utilization percent 0-100.
    pub usage_percent: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    /// Power draw in watts.
    pub power_w: f32,
}

/// One temperature sensor reading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempReading {
    pub label: String,
    pub celsius: f32,
}

/// Per-container resource usage at sample time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStat {
    pub name: String,
    /// CPU usage percent (can exceed 100 on multi-core).
    pub cpu_percent: f32,
    /// Memory used in bytes.
    pub mem_used: u64,
    /// Network bytes received (cumulative since container start).
    pub net_rx: u64,
    /// Network bytes transmitted (cumulative).
    pub net_tx: u64,
}

/// A Kubernetes cluster-state report, pushed by a **cluster-scoped** agent
/// (`AGENT_KIND=k8s-cluster`, one per cluster) that reads the kube-apiserver — as
/// opposed to `MetricsReport`, which a per-node DaemonSet pushes. Carries object
/// counts/health (namespaces, deployments, pods), not host metrics. Authenticated
/// with the same `x-api-key` header (the key encodes the RBAC workspace); posted to
/// `POST /pub/kube` and answered with an `IngestAck` (same interval-ramp contract).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeReport {
    /// Unix epoch seconds when the snapshot was taken on the agent.
    pub ts: i64,
    /// Cluster name (from `CLUSTER`); groups the objects in the UI.
    pub cluster: String,
    /// Agent binary version (CARGO_PKG_VERSION).
    #[serde(default)]
    pub agent_version: String,
    /// Kubernetes server version from the apiserver `/version` (`gitVersion`, e.g.
    /// "v1.29.4"). Empty if unavailable. This is the CLUSTER's k8s version, distinct
    /// from `agent_version`.
    #[serde(default)]
    pub k8s_version: String,
    /// One entry per Kubernetes namespace, with pod-phase tallies.
    #[serde(default)]
    pub namespaces: Vec<KubeNamespaceStat>,
    /// One entry per Deployment, with replica health.
    #[serde(default)]
    pub deployments: Vec<KubeDeploymentStat>,
    /// One entry per **container** across all pods, with usage + pod metadata.
    /// The hub aggregates these on read (by namespace / workload / label).
    #[serde(default)]
    pub containers: Vec<KubeContainerStat>,
}

/// Pod tallies for one Kubernetes namespace at snapshot time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeNamespaceStat {
    /// Kubernetes namespace name (NOT the RBAC workspace).
    pub name: String,
    /// Namespace lifecycle phase: "Active" or "Terminating".
    #[serde(default)]
    pub phase: String,
    pub pods_total: u32,
    pub pods_running: u32,
    pub pods_pending: u32,
    pub pods_failed: u32,
    pub pods_succeeded: u32,
    /// Sum of container restart counts across the namespace's pods.
    #[serde(default)]
    pub restarts: u32,
}

/// Replica health for one Deployment at snapshot time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeDeploymentStat {
    /// Kubernetes namespace the Deployment lives in.
    pub namespace: String,
    pub name: String,
    /// `.spec.replicas` — desired replica count.
    pub desired: u32,
    /// `.status.readyReplicas`.
    pub ready: u32,
    /// `.status.availableReplicas`.
    pub available: u32,
    /// `.status.updatedReplicas`.
    pub updated: u32,
}

/// Per-container resource usage + pod metadata at snapshot time — the granular
/// unit the cluster agent reports so the hub can aggregate on read by any
/// dimension (namespace, workload, or label), rather than pre-aggregating.
///
/// One entry per **container** (a pod with N containers yields N entries), each
/// carrying its pod's identity/metadata duplicated. CPU/memory come from
/// metrics-server (`metrics.k8s.io`); they're 0 when metrics-server is absent
/// (the metadata is still reported). Optional on the wire (`#[serde(default)]`)
/// so older agents/hubs interoperate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeContainerStat {
    /// Kubernetes namespace the pod lives in.
    pub namespace: String,
    /// Pod name.
    pub pod: String,
    /// Container name within the pod.
    pub container: String,
    /// Node the pod is scheduled on (`.spec.nodeName`); empty if unscheduled.
    #[serde(default)]
    pub node: String,
    /// Pod lifecycle phase: Running / Pending / Failed / Succeeded / Unknown.
    #[serde(default)]
    pub phase: String,
    /// Resolved top-level controller name (Deployment/StatefulSet/DaemonSet/Job…),
    /// walking pod → ReplicaSet → Deployment. Empty for bare pods.
    #[serde(default)]
    pub workload: String,
    /// Kind of `workload` (e.g. "Deployment", "StatefulSet", "DaemonSet", "Job",
    /// "ReplicaSet" when the RS has no controller, or "" for a bare pod).
    #[serde(default)]
    pub workload_kind: String,
    /// Current CPU usage in **millicores** (metrics-server); 0 if unavailable.
    #[serde(default)]
    pub cpu_millicores: u64,
    /// Current memory usage in **bytes** (metrics-server); 0 if unavailable.
    #[serde(default)]
    pub mem_bytes: u64,
    /// Container restart count (`.status.containerStatuses[].restartCount`).
    #[serde(default)]
    pub restarts: u32,
    /// Full pod labels, for aggregating by label (e.g. `app`, `team`). Stored as
    /// a JSON object; duplicated across the pod's containers.
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// Hub's response to a successful ingest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestAck {
    pub ok: bool,
    /// Suggested interval (seconds) for the agent's next report.
    pub next_interval_secs: u64,
    /// Hub's build id (short git sha). Agents on the `auto` release channel compare
    /// this to their own build and self-restart (so k8s re-pulls `:auto-update`)
    /// when it differs — letting the fleet follow the hub. Optional for wire
    /// back-compat: older hubs omit it, older agents ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_build: Option<String>,
}

/// Header the agent uses to present its API key.
pub const API_KEY_HEADER: &str = "x-api-key";
