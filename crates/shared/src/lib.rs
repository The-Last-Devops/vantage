//! Types shared between the agent and the hub.

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
    /// Per-core CPU usage % (htop-style); empty if unavailable.
    #[serde(default)]
    pub cpu_per_core: Vec<f32>,
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

/// Hub's response to a successful ingest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestAck {
    pub ok: bool,
    /// Suggested interval (seconds) for the agent's next report.
    pub next_interval_secs: u64,
}

/// Header the agent uses to present its API key.
pub const API_KEY_HEADER: &str = "x-api-key";
