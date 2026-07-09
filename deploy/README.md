# Deploying vantage

The hub and agents install **separately**. The hub is the central server (UI + API
+ databases); agents run on the machines you want to monitor and **push** metrics to
the hub. You enroll each agent with a token created in the UI under **Add System**.

## Local dev
`docker compose up -d` (hub + databases + a bundled agent for convenience). UI at
http://localhost:8080.

## Docker (single host, published images)

Run the hub + databases with the released images — no clone, no build. The hub needs
**two databases** on one TimescaleDB instance: `vantage_config` and `vantage_data`
(the hub enables the TimescaleDB extension in `vantage_data` on first start).

```yaml
# compose.yaml
services:
  db:
    image: timescale/timescaledb:latest-pg18
    environment:
      POSTGRES_USER: vantage
      POSTGRES_PASSWORD: ${DB_PASSWORD:?set a strong password}
      POSTGRES_DB: vantage_config
      PGDATA: /var/lib/postgresql/data/pgdata
    volumes:
      - db:/var/lib/postgresql/data
      - ./init-data-db.sh:/docker-entrypoint-initdb.d/10-data-db.sh:ro  # creates the 2nd DB
    healthcheck: { test: ["CMD-SHELL", "pg_isready -U vantage"], interval: 5s, retries: 10 }

  hub:
    image: ghcr.io/the-last-devops/vantage-hub:latest   # or :<version>
    depends_on: { db: { condition: service_healthy } }
    environment:
      CONFIG_DATABASE_URL: postgres://vantage:${DB_PASSWORD}@db:5432/vantage_config
      DATA_DATABASE_URL:   postgres://vantage:${DB_PASSWORD}@db:5432/vantage_data
      BIND_ADDR: 0.0.0.0:8080
      ADMIN_EMAIL: ${ADMIN_EMAIL:-admin@local}      # first admin (or use the setup screen)
      ADMIN_PASSWORD: ${ADMIN_PASSWORD:?set a strong password}
    ports: ["8080:8080"]
    cap_add: [NET_RAW]            # for ICMP "ping" monitors
    restart: unless-stopped
volumes: { db: {} }
```

```bash
# init-data-db.sh — creates the data DB next to the config DB
printf '#!/bin/bash\nset -e\npsql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "CREATE DATABASE vantage_data;"\n' > init-data-db.sh
DB_PASSWORD=$(openssl rand -hex 16) ADMIN_PASSWORD=changeme docker compose up -d
```

The hub runs migrations automatically on start; UI/API at `http://localhost:8080` (put it
behind a TLS reverse proxy in production). Then **Add System** to install agents (below).
Upgrade with `docker compose pull && docker compose up -d`. The hub image is **amd64-only**;
the agent image is multi-arch.

## 1. Install the hub (Helm)
`deploy/chart` installs the hub plus its two databases (config on plain Postgres,
data on TimescaleDB), each with its own PVC.

> **v3 needs a FRESH database.** Migrations were squashed into a single consolidated
> schema at v3.0.0, so v3 will NOT start against an existing 2.x database (sqlx refuses
> with "migration 1 … has been modified"). New installs only.

```bash
helm install lm ./deploy/chart \
  --namespace vantage --create-namespace \
  --set hub.ingress.host=monitor.senprints.net \
  --set timescaledb.storageClass=sp-hostpath
```

Sensible defaults (nothing required):
- **First admin** is created from the UI on first visit (one-time setup screen).
  Pre-seed instead with `--set admin.email=you@co --set admin.password=secret`.
- **DB password** is auto-generated and kept stable across `helm upgrade`.
- **EXEC_APP_SECRET** (encrypts SSH-key material) is auto-generated into the release
  Secret `<release>` (key `app-secret`) — **back it up**; losing it makes stored SSH
  keys unrecoverable. Disable with `--set hub.autoAppSecret=false`.
- Hub runs **non-root** (uid 10001) with only `CAP_NET_RAW` (for ICMP ping), resource
  limits, a liveness probe, and is pinned to amd64 nodes (the hub image is amd64-only).

Production hardening (opt-in):
- **Backups**: `--set backup.enabled=true` (nightly pg_dump of both DBs → a PVC).
- **NetworkPolicy** (lock the bundled DBs to the hub): `--set networkPolicy.enabled=true`.
- **PodDisruptionBudget** (with `hub.replicas>1`): `--set podDisruptionBudget.enabled=true`.
- **Auto-update** (rolling `:auto-update` channel, self-redeploys): `--set autoUpdate=true`.

### Expose the UI
- Domain (nginx ingress by default): `--set hub.ingress.host=monitor.senprints.net` (host alone enables it)
  - Other controller: `--set hub.ingress.className=traefik`
  - HTTPS via cert-manager: `--set hub.ingress.tls=true --set hub.ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt`
- Or `--set hub.service.type=NodePort` / `LoadBalancer`, or
  `kubectl -n vantage port-forward svc/vantage-hub 8080:8080`.

### External DB instead of in-cluster
`--set timescaledb.enabled=false --set hub.configDatabaseUrl=... --set hub.dataDatabaseUrl=...`

## 2. Add a system (get an API key)
In the UI: **Add System** → choose workspace/kind → copy the **API key**.
One key enrolls a whole DaemonSet; each node shows up under **Kubernetes › <cluster>**.

## 3. Install the agent

**Easiest — the hub serves a ready-to-apply manifest** (this is what the UI shows):
```bash
kubectl apply -f "https://monitor.senprints.net/pub/agent.yaml?key=<api-key>&cluster=k8s-hanoi"
```
The hub fills in its own URL, the key, and the cluster — no clone, no chart registry.

The served manifest installs BOTH the per-node DaemonSet (host metrics) and the
one-per-cluster collector (namespace / deployment / pod / **per-container CPU-RAM** —
the **Clusters** page). Container CPU/RAM needs **metrics-server** in the cluster;
without it the metadata still populates and usage reads 0.

**Helm** (if you prefer a release you manage) — `deploy/agent` installs the DaemonSet
**and** the cluster-agent (Deployment + read-only ClusterRole):
```bash
helm install vantage-agent ./deploy/agent \
  --namespace vantage --create-namespace \
  --set hubUrl=https://monitor.senprints.net \
  --set apiKey=<api-key-from-Add-System> \
  --set cluster=k8s-hanoi
# same cluster as the hub? use the in-cluster Service: --set hubUrl=http://vantage-hub.vantage:8080
# opt into the reverse SSH tunnel (off by default): --set allowShell=true
# per-node host metrics only (skip the cluster collector): --set clusterAgent.enabled=false
```

For a single host outside k8s: `curl -fsSL https://monitor.senprints.net/pub/install.sh | HUB_URL=… API_KEY=… sh`
(native binary + systemd), or run the agent container directly.

## Images
`ghcr.io/the-last-devops/vantage-{hub,agent}` — tagged releases publish `:<version>`
(e.g. `:3.0.0`) + `:latest`; `:main` is the rolling build from `main`; `:auto-update`
is the self-updating channel. The chart pins `:3.0.0` by default. If the packages are
private:
```bash
kubectl -n vantage create secret docker-registry ghcr \
  --docker-server=ghcr.io --docker-username=<gh-user> --docker-password=<PAT read:packages>
helm ... --set image.pullSecrets='{ghcr}'   # agent chart: same flag
```
