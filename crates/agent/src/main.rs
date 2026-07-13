//! vantage agent: collects system metrics and pushes them to the hub.
//!
//! Configuration via environment variables:
//!   HUB_URL      e.g. http://hub.example.com:8080  (required)
//!   API_KEY      API key issued by the hub (required)
//!   INTERVAL     initial report interval, seconds (default 60; hub ramps it live)

mod collect;
mod kube;
mod push;
mod tunnel;

use std::time::Duration;

use anyhow::{Context, Result};
use sysinfo::{Components, Networks, System};

use collect::{collect, collect_containers, collect_gpus, disk_io_ticks, read_cpu_times};
use push::{send_report, shutdown_signal, upgrade_target, Sent};

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

    // Cluster-scoped Kubernetes collector is a wholly separate loop: it talks to the
    // kube-apiserver, not sysinfo/Docker. One per cluster (a Deployment), so branch
    // off before any host-metrics setup and never touch that machinery.
    if cfg.kind == "k8s-cluster" {
        return kube::run(&cfg).await;
    }

    // Don't follow redirects: a http→https redirect (301/302) would silently turn
    // our POST into a GET and the hub would answer 405. Surface it instead so the
    // user fixes HUB_URL to https.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("vantage-agent/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut ingest_url = format!("{}/pub/ingest", cfg.hub_url);

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

    // Shell/exec reverse tunnel (opt-in via ALLOW_SHELL). Forward-only, loopback
    // only — see tunnel.rs / docs/exec-design.md. Off by default.
    if tunnel::enabled() {
        // tungstenite's rustls TLS builder reads the process-default crypto provider.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let host = if cfg.hostname_override.is_empty() {
            System::host_name().unwrap_or_default()
        } else {
            cfg.hostname_override.clone()
        };
        tracing::warn!(%host, "ALLOW_SHELL=1 — opening reverse shell tunnel to the hub");
        tokio::spawn(tunnel::run(cfg.hub_url.clone(), cfg.api_key.clone(), host));
    }

    let mut shutdown = Box::pin(shutdown_signal());
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
        let mut outcome = send_report(&client, &ingest_url, &cfg.api_key, &report).await;
        // Self-heal the common "HUB_URL is http but the hub sits behind TLS" case:
        // on a redirect, upgrade http→https IF it stays on the SAME host (a relative
        // Location, or one pointing at our own https origin). We never follow a
        // redirect to a different host — that could leak the agent token.
        if let Sent::Redirect(ref loc) = outcome {
            if let Some(https_url) = upgrade_target(&ingest_url, loc) {
                tracing::warn!("hub redirected http→https — upgrading to https and retrying");
                ingest_url = https_url;
                outcome = send_report(&client, &ingest_url, &cfg.api_key, &report).await;
            }
        }
        match outcome {
            Sent::Ok { next } => {
                if let Some(secs) = next {
                    interval = Duration::from_secs(secs);
                }
                // Updates are driven externally (GitOps / rolling the image tag);
                // the agent no longer self-restarts on a newer hub build.
                tracing::debug!(cpu = report.cpu_percent, "report sent");
            }
            Sent::Redirect(loc) => tracing::warn!(
                location = %loc,
                "hub redirected the request — set HUB_URL to the final URL (likely https://)"
            ),
            Sent::Rejected(status) => tracing::warn!(%status, "hub rejected report"),
            Sent::Failed => tracing::warn!("failed to reach hub"),
        }
        // Wait out the interval, but wake immediately on a stop signal so the
        // agent exits cleanly (no half-cycle left hanging) instead of being killed.
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received — agent stopping");
                break;
            }
        }
    }
    Ok(())
}
