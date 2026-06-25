# Deploying last-monitor

The hub and agents install **separately**. The hub is the central server (UI + API
+ databases); agents run on the machines you want to monitor and **push** metrics to
the hub. You enroll each agent with a token created in the UI under **Add System**.

## Local dev
`docker compose up -d` (hub + databases + a bundled agent for convenience). UI at
http://localhost:8080.

## Docker (single host, published images)

Run the hub + databases with the released images — no clone, no build. The hub needs
**two databases** on one TimescaleDB instance: `lastmon_config` and `lastmon_data`
(the hub enables the TimescaleDB extension in `lastmon_data` on first start).

```yaml
# compose.yaml
services:
  db:
    image: timescale/timescaledb:latest-pg18
    environment:
      POSTGRES_USER: lastmon
      POSTGRES_PASSWORD: ${DB_PASSWORD:?set a strong password}
      POSTGRES_DB: lastmon_config
      PGDATA: /var/lib/postgresql/data/pgdata
    volumes:
      - db:/var/lib/postgresql/data
      - ./init-data-db.sh:/docker-entrypoint-initdb.d/10-data-db.sh:ro  # creates the 2nd DB
    healthcheck: { test: ["CMD-SHELL", "pg_isready -U lastmon"], interval: 5s, retries: 10 }

  hub:
    image: ghcr.io/the-last-devops/last-monitor-hub:latest   # or :<version>
    depends_on: { db: { condition: service_healthy } }
    environment:
      CONFIG_DATABASE_URL: postgres://lastmon:${DB_PASSWORD}@db:5432/lastmon_config
      DATA_DATABASE_URL:   postgres://lastmon:${DB_PASSWORD}@db:5432/lastmon_data
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
printf '#!/bin/bash\nset -e\npsql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c "CREATE DATABASE lastmon_data;"\n' > init-data-db.sh
DB_PASSWORD=$(openssl rand -hex 16) ADMIN_PASSWORD=changeme docker compose up -d
```

The hub runs migrations automatically on start; UI/API at `http://localhost:8080` (put it
behind a TLS reverse proxy in production). Then **Add System** to install agents (below).
Upgrade with `docker compose pull && docker compose up -d`. The hub image is **amd64-only**;
the agent image is multi-arch.

## 1. Install the hub (Helm)
`deploy/chart` installs the hub plus its two databases (config on plain Postgres,
data on TimescaleDB), each with its own PVC.

```bash
helm install lm ./deploy/chart \
  --namespace last-monitor --create-namespace \
  --set hub.ingress.host=monitor.senprints.net \
  --set timescaledb.storageClass=sp-hostpath
```

Nothing required to set:
- **First admin** is created from the UI on first visit (one-time setup screen).
  Pre-seed instead with `--set admin.email=you@co --set admin.password=secret`.
- **DB password** is auto-generated and kept stable across `helm upgrade`.

### Expose the UI
- Domain (nginx ingress by default): `--set hub.ingress.host=monitor.senprints.net` (host alone enables it)
  - Other controller: `--set hub.ingress.className=traefik`
  - HTTPS via cert-manager: `--set hub.ingress.tls=true --set hub.ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt`
- Or `--set hub.service.type=NodePort` / `LoadBalancer`, or
  `kubectl -n last-monitor port-forward svc/lm-hub 8080:8080`.

### External DB instead of in-cluster
`--set timescaledb.enabled=false --set hub.configDatabaseUrl=... --set hub.dataDatabaseUrl=...`

## 2. Add a system (get an API key)
In the UI: **Add System** → choose namespace/kind → copy the **API key**.
One key enrolls a whole DaemonSet; each node shows up under **Kubernetes › <cluster>**.

## 3. Install the agent

**Easiest — the hub serves a ready-to-apply manifest** (this is what the UI shows):
```bash
kubectl apply -f "https://monitor.senprints.net/pub/agent.yaml?key=<api-key>&cluster=k8s-hanoi"
```
The hub fills in its own URL, the key, and the cluster — no clone, no chart registry.

**Helm** (if you prefer a release you manage) — `deploy/agent` is the same DaemonSet:
```bash
helm install lm-agent ./deploy/agent \
  --namespace last-monitor --create-namespace \
  --set hubUrl=https://monitor.senprints.net \
  --set apiKey=<api-key-from-Add-System> \
  --set cluster=k8s-hanoi
# same cluster as the hub? use the in-cluster Service: --set hubUrl=http://lm-hub.last-monitor:8080
```

For a single host outside k8s: `curl -fsSL https://monitor.senprints.net/pub/install.sh | HUB_URL=… API_KEY=… sh`
(native binary + systemd), or run the agent container directly.

## Images
`ghcr.io/the-last-devops/last-monitor-{hub,agent}:main` (rolling latest from `main`;
tagged releases also publish `:<version>` and `:latest`). If the packages are private:
```bash
kubectl -n last-monitor create secret docker-registry ghcr \
  --docker-server=ghcr.io --docker-username=<gh-user> --docker-password=<PAT read:packages>
helm ... --set image.pullSecrets='{ghcr}'   # agent chart: same flag
```
