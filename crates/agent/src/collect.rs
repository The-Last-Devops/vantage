//! System metric collection via `sysinfo`, `/proc`, mach, `nvidia-smi`, and Docker.

use shared::{ContainerStat, GpuStat, MetricsReport, TempReading};
use sysinfo::{Components, Networks, System};

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

pub fn collect(
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
    let (mem_available, mem_buffers, mem_cached, mem_free) = mem_detail(sys);

    let load = System::load_average();
    MetricsReport {
        ts: now_secs(),
        hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
        cpu_percent,
        mem_used: sys.used_memory(),
        mem_total: sys.total_memory(),
        swap_used: sys.used_swap(),
        swap_total: sys.total_swap(),
        mem_available,
        mem_buffers,
        mem_cached,
        mem_free,
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

/// Memory breakdown (available, buffers, cached, free) in bytes. On Linux this parses
/// `/proc/meminfo` (kB → bytes; `cached` = Cached + SReclaimable). Elsewhere only
/// `available` is known (from sysinfo) and the rest are 0.
#[cfg(target_os = "linux")]
fn mem_detail(_sys: &System) -> (u64, u64, u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0, 0, 0);
    };
    let (mut avail, mut buffers, mut cached, mut sreclaim, mut free) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for line in content.lines() {
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let val: u64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemAvailable:" => avail = val,
            "MemFree:" => free = val,
            "Buffers:" => buffers = val,
            "Cached:" => cached = val,
            "SReclaimable:" => sreclaim = val,
            _ => {}
        }
    }
    (
        avail * 1024,
        buffers * 1024,
        (cached + sreclaim) * 1024,
        free * 1024,
    )
}

#[cfg(not(target_os = "linux"))]
fn mem_detail(sys: &System) -> (u64, u64, u64, u64) {
    (sys.available_memory(), 0, 0, 0)
}

/// Per whole-disk cumulative "time spent doing I/O" in ms (/proc/diskstats field
/// 13, io_ticks). Empty off Linux. Used to derive iostat-style %util.
pub fn disk_io_ticks() -> std::collections::HashMap<String, u64> {
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
pub struct CpuTimes {
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
pub fn read_cpu_times() -> Option<CpuTimes> {
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
pub fn read_cpu_times() -> Option<CpuTimes> {
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
pub fn read_cpu_times() -> Option<CpuTimes> {
    None
}

impl CpuTimes {
    /// (user, system, iowait, steal) as percentages of the delta since `prev`.
    pub fn delta_pct(&self, prev: &CpuTimes) -> (f32, f32, f32, f32) {
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
pub async fn collect_gpus() -> Vec<GpuStat> {
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
pub async fn collect_containers(docker: &bollard::Docker) -> Vec<ContainerStat> {
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

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{disk_usage, is_whole_disk};

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

    #[test]
    fn whole_disk_vs_partition() {
        assert!(is_whole_disk("sda"));
        assert!(!is_whole_disk("sda1"));
        assert!(is_whole_disk("vda"));
        assert!(!is_whole_disk("vda2"));
        assert!(is_whole_disk("xvda"));
        assert!(!is_whole_disk("xvda1"));
        assert!(is_whole_disk("nvme0n1"));
        assert!(!is_whole_disk("nvme0n1p1"));
        assert!(is_whole_disk("mmcblk0"));
        assert!(!is_whole_disk("mmcblk0p1"));
        assert!(!is_whole_disk("loop0"));
        assert!(!is_whole_disk("dm-0"));
    }
}
