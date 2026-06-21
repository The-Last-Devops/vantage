//! last-monitor agent: collects system metrics and pushes them to the hub.
//!
//! Configuration via environment variables:
//!   HUB_URL      e.g. http://hub.example.com:8080  (required)
//!   AGENT_TOKEN  enrollment token issued by the hub (required)
//!   INTERVAL     report interval in seconds (default 15)
#![allow(clippy::items_after_test_module)]

use std::time::Duration;

use anyhow::{Context, Result};
use shared::{ContainerStat, GpuStat, IngestAck, MetricsReport, TempReading, API_KEY_HEADER};
use sysinfo::{Components, Networks, System};

struct Config {
    hub_url: String,
    token: String,
    interval: Duration,
    /// Filesystem path whose usage we report. In Docker, mount the host root and
    /// set DISK_PATH=/host so the host's disk (not the container's) is measured.
    disk_path: String,
    /// System kind: "node" (default), "docker", or "k8s" (set via AGENT_KIND).
    kind: String,
    /// Cluster name for k8s nodes (set via CLUSTER); empty otherwise.
    cluster: String,
    /// Override the reported hostname (set via HOSTNAME_OVERRIDE or NODE_NAME).
    /// On k8s the DaemonSet injects the node name so identity is stable per node
    /// (not the ephemeral pod name).
    hostname_override: String,
}

fn load_config() -> Result<Config> {
    let hub_url = std::env::var("HUB_URL").context("HUB_URL is required")?;
    let token = std::env::var("AGENT_TOKEN").context("AGENT_TOKEN is required")?;
    let interval = std::env::var("INTERVAL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(15);
    let disk_path = std::env::var("DISK_PATH").unwrap_or_else(|_| "/".to_string());
    let kind = std::env::var("AGENT_KIND").unwrap_or_else(|_| "node".to_string());
    let cluster = std::env::var("CLUSTER").unwrap_or_default();
    let hostname_override = std::env::var("HOSTNAME_OVERRIDE")
        .or_else(|_| std::env::var("NODE_NAME"))
        .unwrap_or_default();
    Ok(Config {
        hub_url: hub_url.trim_end_matches('/').to_string(),
        token,
        interval: Duration::from_secs(interval.max(1)),
        disk_path,
        kind,
        cluster,
        hostname_override,
    })
}

/// Disk usage (used, total) in bytes for a path, via statvfs. Predictable in
/// containers (unlike enumerating mounts, which sees the container's view).
///
/// statvfs field widths differ by platform (u64 on Linux, u32 on macOS), so the
/// `as u64` casts are necessary on some targets even if redundant on others.
#[allow(clippy::unnecessary_cast)]
fn disk_usage(path: &str) -> (u64, u64) {
    match nix::sys::statvfs::statvfs(path) {
        Ok(s) => {
            let frsize = s.fragment_size() as u64;
            let total = s.blocks() as u64 * frsize;
            let avail = s.blocks_available() as u64 * frsize;
            (total.saturating_sub(avail), total)
        }
        Err(e) => {
            tracing::warn!(error = %e, path, "statvfs failed");
            (0, 0)
        }
    }
}

fn collect(
    sys: &mut System,
    nets: &mut Networks,
    components: &mut Components,
    disk_path: &str,
) -> MetricsReport {
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    nets.refresh(false);
    components.refresh(false);

    let cpu_percent = sys.global_cpu_usage();
    let (disk_used, disk_total) = disk_usage(disk_path);

    let (mut net_rx, mut net_tx) = (0u64, 0u64);
    for data in nets.list().values() {
        net_rx += data.total_received();
        net_tx += data.total_transmitted();
    }

    let temps: Vec<TempReading> = components
        .list()
        .iter()
        .filter_map(|c| {
            c.temperature()
                .filter(|t| t.is_finite())
                .map(|t| TempReading {
                    label: c.label().to_string(),
                    celsius: t,
                })
        })
        .collect();

    let (disk_read, disk_write) = disk_io();

    MetricsReport {
        ts: now_secs(),
        hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
        cpu_percent,
        mem_used: sys.used_memory(),
        mem_total: sys.total_memory(),
        swap_used: sys.used_swap(),
        swap_total: sys.total_swap(),
        disk_used,
        disk_total,
        net_rx,
        net_tx,
        load1: System::load_average().one,
        uptime: System::uptime(),
        kind: String::new(), // set from config in the push loop
        cluster: String::new(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        kernel: System::kernel_version().unwrap_or_default(),
        cpu_model: sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default(),
        cpu_cores: sys.cpus().len() as u32,
        disk_read,
        disk_write,
        temps,
        containers: Vec::new(),
        gpus: Vec::new(),
    }
}

/// True for whole disks (not partitions) in /proc/diskstats.
fn is_whole_disk(name: &str) -> bool {
    if name.starts_with("nvme") || name.starts_with("mmcblk") {
        return !name.contains('p');
    }
    if name.starts_with("sd")
        || name.starts_with("vd")
        || name.starts_with("xvd")
        || name.starts_with("hd")
    {
        return !name
            .chars()
            .last()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(true);
    }
    false
}

/// Cumulative disk bytes (read, write) from /proc/diskstats. (0,0) off Linux.
fn disk_io() -> (u64, u64) {
    let content = match std::fs::read_to_string("/proc/diskstats") {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let (mut r, mut w) = (0u64, 0u64);
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || !is_whole_disk(f[2]) {
            continue;
        }
        // sectors read = field 5, sectors written = field 9; 512 bytes/sector.
        if let (Ok(sr), Ok(sw)) = (f[5].parse::<u64>(), f[9].parse::<u64>()) {
            r += sr * 512;
            w += sw * 512;
        }
    }
    (r, w)
}

/// Per-GPU stats via `nvidia-smi`. Empty if unavailable (no NVIDIA GPU / tool).
async fn collect_gpus() -> Vec<GpuStat> {
    let out = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await;
    let Ok(o) = out else { return Vec::new() };
    if !o.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let mib = 1024.0 * 1024.0;
    text.lines()
        .filter_map(|line| {
            let p: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if p.len() < 5 {
                return None;
            }
            Some(GpuStat {
                name: p[0].to_string(),
                usage_percent: p[1].parse().unwrap_or(0.0),
                mem_used: (p[2].parse::<f64>().unwrap_or(0.0) * mib) as u64,
                mem_total: (p[3].parse::<f64>().unwrap_or(0.0) * mib) as u64,
                power_w: p[4].parse().unwrap_or(0.0),
            })
        })
        .collect()
}

/// Per-container stats via the Docker API (unix socket). Returns empty if Docker
/// is unreachable. CPU% uses the standard delta-over-system-delta formula.
async fn collect_containers(docker: &bollard::Docker) -> Vec<ContainerStat> {
    use bollard::container::{ListContainersOptions, StatsOptions};
    use futures_util::StreamExt;

    let list = match docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        }))
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(error = %e, "docker list");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for c in list {
        let Some(id) = c.id else { continue };
        let name = c
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_else(|| id.chars().take(12).collect());

        let mut stream = docker.stats(
            &id,
            Some(StatsOptions {
                stream: false,
                one_shot: false,
            }),
        );
        let Some(Ok(s)) = stream.next().await else {
            continue;
        };

        let cpu_delta =
            s.cpu_stats.cpu_usage.total_usage as f64 - s.precpu_stats.cpu_usage.total_usage as f64;
        let sys_delta = s.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
            - s.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;
        let ncpu = s.cpu_stats.online_cpus.unwrap_or(1).max(1) as f64;
        let cpu_percent = if sys_delta > 0.0 && cpu_delta > 0.0 {
            (cpu_delta / sys_delta * ncpu * 100.0) as f32
        } else {
            0.0
        };

        let (mut net_rx, mut net_tx) = (0u64, 0u64);
        if let Some(nets) = &s.networks {
            for n in nets.values() {
                net_rx += n.rx_bytes;
                net_tx += n.tx_bytes;
            }
        }

        out.push(ContainerStat {
            name,
            cpu_percent,
            mem_used: s.memory_stats.usage.unwrap_or(0),
            net_rx,
            net_tx,
        });
    }
    out
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::disk_usage;

    #[test]
    fn disk_usage_root_is_sane() {
        // "/" always exists; used must not exceed total and total must be > 0.
        let (used, total) = disk_usage("/");
        assert!(total > 0, "total should be positive");
        assert!(used <= total, "used must be <= total");
    }

    #[test]
    fn disk_usage_bad_path_is_zero() {
        assert_eq!(disk_usage("/no/such/path/here"), (0, 0));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = load_config()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let ingest_url = format!("{}/api/ingest", cfg.hub_url);

    let mut sys = System::new_all();
    let mut nets = Networks::new_with_refreshed_list();
    let mut components = Components::new_with_refreshed_list();

    // Connect to the local Docker daemon if available (container stats).
    let docker = match bollard::Docker::connect_with_local_defaults() {
        Ok(d) => match d.ping().await {
            Ok(_) => {
                tracing::info!("docker connected — collecting container stats");
                Some(d)
            }
            Err(e) => {
                tracing::info!(error = %e, "docker not reachable — skipping container stats");
                None
            }
        },
        Err(e) => {
            tracing::info!(error = %e, "docker not configured — skipping container stats");
            None
        }
    };

    // First CPU reading is meaningless; prime it then wait briefly.
    sys.refresh_cpu_usage();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut interval = cfg.interval;
    tracing::info!(hub = %cfg.hub_url, ?interval, "agent started");

    loop {
        let mut report = collect(&mut sys, &mut nets, &mut components, &cfg.disk_path);
        report.kind = cfg.kind.clone();
        report.cluster = cfg.cluster.clone();
        if !cfg.hostname_override.is_empty() {
            report.hostname = cfg.hostname_override.clone();
        }
        if let Some(d) = &docker {
            report.containers = collect_containers(d).await;
        }
        report.gpus = collect_gpus().await;
        match client
            .post(&ingest_url)
            .header(API_KEY_HEADER, &cfg.token)
            .json(&report)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(ack) = resp.json::<IngestAck>().await {
                    if ack.next_interval_secs > 0 {
                        interval = Duration::from_secs(ack.next_interval_secs);
                    }
                }
                tracing::debug!(cpu = report.cpu_percent, "report sent");
            }
            Ok(resp) => tracing::warn!(status = %resp.status(), "hub rejected report"),
            Err(e) => tracing::warn!(error = %e, "failed to reach hub"),
        }
        tokio::time::sleep(interval).await;
    }
}
