//! last-monitor agent: collects system metrics and pushes them to the hub.
//!
//! Configuration via environment variables:
//!   HUB_URL      e.g. http://hub.example.com:8080  (required)
//!   API_KEY      API key issued by the hub (required)
//!   INTERVAL     initial report interval, seconds (default 60; hub ramps it live)
#![allow(clippy::items_after_test_module)]

use std::time::Duration;

use anyhow::{Context, Result};
use shared::{ContainerStat, GpuStat, IngestAck, MetricsReport, TempReading, API_KEY_HEADER};
use sysinfo::{Components, Networks, System};

struct Config {
    hub_url: String,
    api_key: String,
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
    let api_key = std::env::var("API_KEY").context("API_KEY is required")?;
    // Default to 1-minute pushes (matches the smallest stored rollup; low load).
    // The hub ramps this down to near-realtime via IngestAck while a host is being
    // viewed, then back up. INTERVAL only overrides the initial value.
    let interval = std::env::var("INTERVAL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60);
    let disk_path = std::env::var("DISK_PATH").unwrap_or_else(|_| "/".to_string());
    let kind = std::env::var("AGENT_KIND").unwrap_or_else(|_| "node".to_string());
    let cluster = std::env::var("CLUSTER").unwrap_or_default();
    let hostname_override = std::env::var("HOSTNAME_OVERRIDE")
        .or_else(|_| std::env::var("NODE_NAME"))
        .unwrap_or_default();
    Ok(Config {
        hub_url: hub_url.trim_end_matches('/').to_string(),
        api_key,
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

    let load = System::load_average();
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
        load1: load.one,
        load5: load.five,
        load15: load.fifteen,
        // CPU breakdown is filled in by the push loop (needs prev /proc/stat).
        cpu_user: 0.0,
        cpu_system: 0.0,
        cpu_iowait: 0.0,
        cpu_steal: 0.0,
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
        disk_util: 0.0, // filled in the push loop from the io_ticks delta
        temps,
        containers: Vec::new(),
        gpus: Vec::new(),
    }
}

/// Per whole-disk cumulative "time spent doing I/O" in ms (/proc/diskstats field
/// 13, io_ticks). Empty off Linux. Used to derive iostat-style %util.
fn disk_io_ticks() -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    let Ok(content) = std::fs::read_to_string("/proc/diskstats") else {
        return out;
    };
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 13 || !is_whole_disk(f[2]) {
            continue;
        }
        if let Ok(ticks) = f[12].parse::<u64>() {
            out.insert(f[2].to_string(), ticks);
        }
    }
    out
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

/// Cumulative CPU jiffies from /proc/stat's aggregate `cpu` line. None off Linux.
struct CpuTimes {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

#[cfg(target_os = "linux")]
fn read_cpu_times() -> Option<CpuTimes> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    let line = content.lines().next()?;
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    let v: Vec<u64> = it.filter_map(|x| x.parse().ok()).collect();
    if v.len() < 8 {
        return None;
    }
    Some(CpuTimes {
        user: v[0],
        nice: v[1],
        system: v[2],
        idle: v[3],
        iowait: v[4],
        irq: v[5],
        softirq: v[6],
        steal: v[7],
    })
}

/// macOS exposes aggregate CPU ticks via mach's HOST_CPU_LOAD_INFO: user / system
/// / idle / nice only — there is no iowait or steal, so those stay 0.
#[cfg(target_os = "macos")]
#[allow(deprecated)] // mach_host_self: fine here; avoids pulling in the mach2 crate
fn read_cpu_times() -> Option<CpuTimes> {
    let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
    let mut count = (std::mem::size_of::<libc::host_cpu_load_info>()
        / std::mem::size_of::<libc::integer_t>())
        as libc::mach_msg_type_number_t;
    let r = unsafe {
        libc::host_statistics(
            libc::mach_host_self(),
            libc::HOST_CPU_LOAD_INFO,
            &mut info as *mut _ as libc::host_info_t,
            &mut count,
        )
    };
    if r != libc::KERN_SUCCESS {
        return None;
    }
    let t = info.cpu_ticks;
    Some(CpuTimes {
        user: t[libc::CPU_STATE_USER as usize] as u64,
        nice: t[libc::CPU_STATE_NICE as usize] as u64,
        system: t[libc::CPU_STATE_SYSTEM as usize] as u64,
        idle: t[libc::CPU_STATE_IDLE as usize] as u64,
        iowait: 0,
        irq: 0,
        softirq: 0,
        steal: 0,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_cpu_times() -> Option<CpuTimes> {
    None
}

impl CpuTimes {
    /// (user, system, iowait, steal) as percentages of the delta since `prev`.
    fn delta_pct(&self, prev: &CpuTimes) -> (f32, f32, f32, f32) {
        let d = |a: u64, b: u64| a.saturating_sub(b);
        let user = d(self.user, prev.user) + d(self.nice, prev.nice);
        let system =
            d(self.system, prev.system) + d(self.irq, prev.irq) + d(self.softirq, prev.softirq);
        let iowait = d(self.iowait, prev.iowait);
        let steal = d(self.steal, prev.steal);
        let idle = d(self.idle, prev.idle);
        let total = user + system + iowait + steal + idle;
        if total == 0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let pct = |x: u64| (x as f32 / total as f32) * 100.0;
        (pct(user), pct(system), pct(iowait), pct(steal))
    }
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

// Single-threaded runtime: the agent's work is light + periodic, so one thread
// keeps the memory/thread footprint minimal (vs. a worker per CPU core).
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = load_config()?;
    // Don't follow redirects: a http→https redirect (301/302) would silently turn
    // our POST into a GET and the hub would answer 405. Surface it instead so the
    // user fixes HUB_URL to https.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let ingest_url = format!("{}/pub/ingest", cfg.hub_url);

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
    let mut prev_cpu = read_cpu_times();
    let mut prev_ticks = disk_io_ticks();
    let mut prev_t = std::time::Instant::now();
    tracing::info!(hub = %cfg.hub_url, ?interval, "agent started");

    loop {
        let mut report = collect(&mut sys, &mut nets, &mut components, &cfg.disk_path);
        report.kind = cfg.kind.clone();
        report.cluster = cfg.cluster.clone();
        // Disk %util = busiest disk's io_ticks delta over elapsed wall time (Linux).
        let now = std::time::Instant::now();
        let elapsed_ms = now.duration_since(prev_t).as_millis() as f64;
        let cur_ticks = disk_io_ticks();
        if elapsed_ms > 0.0 {
            let mut util = 0.0f64;
            for (name, ticks) in &cur_ticks {
                if let Some(prev) = prev_ticks.get(name) {
                    let busy = ticks.saturating_sub(*prev) as f64;
                    util = util.max(busy / elapsed_ms * 100.0);
                }
            }
            report.disk_util = util.min(100.0) as f32;
        }
        prev_ticks = cur_ticks;
        prev_t = now;
        // CPU breakdown from the delta between successive /proc/stat reads (Linux).
        let cur_cpu = read_cpu_times();
        if let (Some(prev), Some(cur)) = (&prev_cpu, &cur_cpu) {
            let (u, s, io, st) = cur.delta_pct(prev);
            report.cpu_user = u;
            report.cpu_system = s;
            report.cpu_iowait = io;
            report.cpu_steal = st;
        }
        prev_cpu = cur_cpu;
        if !cfg.hostname_override.is_empty() {
            report.hostname = cfg.hostname_override.clone();
        }
        if let Some(d) = &docker {
            report.containers = collect_containers(d).await;
        }
        report.gpus = collect_gpus().await;
        match client
            .post(&ingest_url)
            .header(API_KEY_HEADER, &cfg.api_key)
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
            Ok(resp) if resp.status().is_redirection() => tracing::warn!(
                status = %resp.status(),
                "hub redirected the request — set HUB_URL to the final URL (likely https://)"
            ),
            Ok(resp) => tracing::warn!(status = %resp.status(), "hub rejected report"),
            Err(e) => tracing::warn!(error = %e, "failed to reach hub"),
        }
        tokio::time::sleep(interval).await;
    }
}
