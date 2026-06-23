<!-- NAMING: the product display name is "Last Monitor" (Title Case) everywhere in
     the UI and docs. The slug "last-monitor" is only for repo / package / image
     names. Do not show "last-monitor" as a user-facing label. -->
<div align="center">

# Last Monitor

**Lightweight, self-hosted server & fleet monitoring — written in Rust.**

Beszel-style host metrics (an agent on every server) with a NewRelic-style fleet overview,
multi-user namespaces and RBAC — all served from a **single Rust binary** that embeds the
web UI. No Node, no `node_modules` at runtime.

</div>

---

## Why

- **One small binary.** The hub serves the JSON API **and** the web UI (a Vue SPA embedded
  into the binary). The whole repo is ~0.25 MB on GitHub; the agent and hub share one Rust
  workspace.
- **Push-based agents.** Agents reach out to the hub, so they work behind NAT/firewalls.
  One reusable **API key** can enroll a whole fleet (e.g. a Kubernetes DaemonSet) — hosts
  auto-register by hostname.
- **Time-series done right.** Metrics live in PostgreSQL + **TimescaleDB** hypertables, kept
  separate from the config database so the time-series store can be scaled independently.

## Features

**Fleet overview** — every host on one chart per metric (CPU / Memory / Disk / Network).
Hover a host to isolate its line across all charts, click to pin (multi-select), drag to
zoom a time range — selection and zoom window are kept in the URL (shareable). A powerful
search box filters both charts and tables: `web*`, `cpu>50`, `ns:production`, `kind:docker`.

**Host metrics** (agent) — overall CPU plus a **CPU breakdown** (user / system / iowait /
steal on Linux via `/proc/stat`; user / system on macOS via mach), **load average** (1m / 5m
/ 15m), memory, swap, disk usage, **disk I/O**, network throughput, uptime, temperature
sensors, NVIDIA **GPU** (usage / VRAM / power), and **per-container Docker stats**.

**Systems view** — nodes, Docker hosts (expand to their containers) and Kubernetes clusters
(expand to their nodes), with a namespace column, sortable columns, multi-select + bulk
delete, and an **Add system** wizard (binary / Docker / Compose / k8s DaemonSet).

**Per-system detail** — uPlot charts with a synced cursor, drag-to-zoom, interactive legends
and live updates (sub-hour ranges refresh every second). Charts always span the selected
window, leaving blank space when data is sparse.

**Services (uptime monitors)** — Uptime-Kuma-style checks: HTTP/HTTPS (status ranges,
keyword match, headers/body/auth, redirects), TCP, Ping, DNS, TLS-cert expiry, **Push**
(passive), and database probes — **PostgreSQL, MySQL, MongoDB, Redis, RabbitMQ**. Full
edit form, retries/flap guard, upside-down mode, and a **last request/response debug**
panel (with copy) for the most recent success and failure. A `Down` view lists only what's
currently failing.

**Alerting** — rules fire a **notification channel** (webhook / Telegram / email) when a
monitor goes down or a host breaches a condition (offline, or CPU/memory/load threshold),
with per-rule cooldown.

**Needs attention** — triage view that surfaces only abnormal hosts (down / high
CPU / memory / disk / disk-I/O), with per-namespace thresholds.

**Admin & data** — an **audit log** of user actions (who/what/when/result), an **About**
page (version, build, update check), and **data retention** tiers you can tune per
downsampling level (TimescaleDB continuous aggregates + retention policies).

**Multi-tenant** — namespaces (k8s-style names), namespace-scoped RBAC plus a system
`admin`, opaque revocable cookie sessions (argon2), and a first-run wizard to create
the admin account. Reusable API keys enroll agents; deleting a key de-registers its hosts.

The sidebar groups everything into **Infrastructure**, **Services**, **Alert** and
**Settings** — click a parent to jump to its first page, hover the arrow to reveal its
sub-pages.

## Architecture

```
                 push (x-api-key)
  ┌─────────┐  ───────────────────────►  ┌──────────────────────────────┐
  │ agent   │     POST /pub/ingest        │  hub (Axum, single binary)   │
  │ (Rust)  │                             │  ingest · auth/RBAC          │
  └─────────┘                             │  JSON API · embedded Vue SPA │
   one per host                           └───────────────┬──────────────┘
                                          ┌───────────────┴───────────────┐
                                          │ config DB (Postgres)           │  users, namespaces,
                                          │ data   DB (Postgres+Timescale) │  RBAC, API keys, systems
                                          └────────────────────────────────┘  metrics, containers
```

Two **separate** PostgreSQL databases (config vs time-series), related only by IDs at the
application layer — **never JOINed** — so the time-series store can be scaled or relocated
independently. The agent ↔ hub wire types live in `crates/shared`. See
[CLAUDE.md](CLAUDE.md) for the full design.

## Quick start (Docker Compose)

```bash
git clone <repo> && cd last-monitor
bash scripts/frontend.sh build        # build the embedded Vue SPA → frontend/dist (first run installs deps)
docker compose up -d --build          # Postgres/TimescaleDB + hub (:8080) + Adminer (:8088) + a bundled agent
```

Open **http://localhost:8080**. On first run you create the admin account (or set
`ADMIN_EMAIL` / `ADMIN_PASSWORD`). A bundled agent reports the Docker host out of the box.

> Want sizeable test data? `bash scripts/sim-agents.sh` spins up many simulated
> node / docker / k8s hosts pushing realistic metrics.

## Adding servers

In the UI: **Add system** → pick Node / Docker / Kubernetes → copy the install snippet. The
API key is managed for you. Run the agent anywhere; hosts appear automatically.

```bash
# Docker (reports host metrics via shared namespaces + mounts)
docker run -d --restart=unless-stopped --pid=host \
  -e HUB_URL=https://hub.example.com -e API_KEY=<api-key> -e DISK_PATH=/host \
  -v /:/host:ro -v /var/run/docker.sock:/var/run/docker.sock:ro \
  ghcr.io/<owner>/last-monitor-agent:latest
```

A **Helm chart** for the hub and a DaemonSet manifest for agents live in [deploy/](deploy/).

## Development

```bash
cargo build                  # whole workspace (hub + agent + shared)
cargo test                   # unit tests
cargo clippy --all-targets   # lint
cargo fmt                    # format

bash scripts/frontend.sh dev # Vite dev server on :5173 (HMR; proxies the API to :8080)
bash scripts/frontend.sh build  # produce frontend/dist embedded by the hub
HUB_URL=http://localhost:8080 API_KEY=<key> cargo run -p agent   # run an agent
```

During UI work use the Vite dev server (**:5173**) — it hot-reloads and is immune to hub
rebuilds. The hub serves the built SPA at **:8080**.

**Stack:** Rust + **Axum** (hub), **sysinfo + bollard** (agent), **sqlx** (runtime queries),
**PostgreSQL + TimescaleDB**, **Vue 3 + Vite + uPlot + Tailwind** SPA embedded via
`rust-embed`.

## Roadmap

Service monitors (HTTP / TCP / ping), alerting (thresholds + webhook/Telegram) and public
status pages are scaffolded in the backend and will return to the UI after the metrics
experience is finalized. TimescaleDB continuous-aggregate rollups + retention are being
wired in (see [docs/](docs/)).

## License

MIT
