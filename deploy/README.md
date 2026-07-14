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

#### Hub environment variables (Docker / any deployment)

Set on the `hub` container. Required unless noted; the rest have safe defaults.

| Variable | Required | Default | Purpose |
|---|:---:|---|---|
| `CONFIG_DATABASE_URL` | ✅ | — | Postgres URL for the **config** DB (users, workspaces, monitors, rules). |
| `DATA_DATABASE_URL` | ✅ | — | Postgres URL for the **data** DB — must have the **TimescaleDB** extension. |
| `ADMIN_EMAIL` | — | `admin@local` | First admin's email. Only used to pre-seed; skip to create the admin on the first-visit setup screen. |
| `ADMIN_PASSWORD` | — | — | First admin's password. Set with `ADMIN_EMAIL` to pre-seed; leave unset to use the setup screen. |
| `BIND_ADDR` | — | `0.0.0.0:8080` | Listen address. |
| `PUBLIC_URL` | — | auto-detected | Externally-reachable base URL (e.g. `https://mon.example.com`); auto-detected from request headers — set only to override (used by passkeys + the exposure self-check). |
| `EXEC_APP_SECRET` | — | — | Encrypts stored SSH-key material at rest (needed for the shell/exec feature). **Back it up** — losing it makes stored keys unrecoverable. |
| `WEBAUTHN_RP_ID` / `WEBAUTHN_ORIGIN` | — | derived from request | Pin the passkey relying-party ID / origin; optional (derived per request otherwise). |
| `INSECURE_COOKIES` | — | `0` | Set `1` to drop the `Secure` cookie flag for **local http dev only**. |
| `EGRESS_POLICY` | — | allow private | Set `strict` to also block private (RFC1918/ULA) outbound targets (SSRF hardening). |
| `LOCAL_API_KEY` | — | — | If set, auto-creates a `default` workspace + a `local` server with this key (lets a bundled agent report out of the box). |
| `RUST_LOG` | — | `info,sqlx=warn` | Log filter (`tracing`/`env_filter`). |

Container-level bits the compose file above sets: `ports: ["8080:8080"]`, `cap_add: [NET_RAW]`
(for ICMP "ping" monitors), a DB volume, and `restart: unless-stopped`.

The **agent** container/binary is configured by its own env — see the agent table in
§3 below.

## 1. Install the hub (Helm)
The chart installs the hub plus its two databases (config on plain Postgres, data on
TimescaleDB), each with its own PVC. It's published as a **public OCI artifact on GHCR**,
so you install straight from the registry — **no `git clone` needed** (Helm ≥ 3.8):

> **v3 needs a FRESH database.** Migrations were squashed into a single consolidated
> schema at v3.0.0, so v3 will NOT start against an existing 2.x database (sqlx refuses
> with "migration 1 … has been modified"). New installs only.

```bash
helm install lm oci://ghcr.io/the-last-devops/charts/vantage --version 3.0.0 \
  --namespace vantage --create-namespace \
  --set hub.ingress.host=monitor.example.com \
  --set timescaledb.storageClass=sp-hostpath
```

> Each release tag publishes `oci://ghcr.io/the-last-devops/charts/vantage` (hub) and
> `…/vantage-agent` (agent) at that version. `helm show values oci://…/vantage --version 3.0.0`
> prints the full defaults. Working from a checkout instead? swap the ref for the local
> path: `helm install lm ./deploy/chart …`.

#### Hub chart values (`deploy/chart`)

Override any of these with `--set key=value` (or a `-f my-values.yaml`). Nothing is
required — the defaults give a working single-node hub with bundled databases.

**Image**
| Key | Default | Purpose |
|---|---|---|
| `image.hub` | `ghcr.io/the-last-devops/vantage-hub` | Hub image repository (no tag). |
| `image.tag` | `3.0.0` | Pinned release tag. Bump + `helm upgrade` to update; or set a moving tag (`latest`/`main`) with `image.pullPolicy=Always`. |
| `image.pullPolicy` | `IfNotPresent` | Set `Always` when tracking a moving tag. |
| `image.pullSecrets` | `[]` | imagePullSecrets for private GHCR, e.g. `{ghcr}`. |

**First admin**
| Key | Default | Purpose |
|---|---|---|
| `admin.email` | `admin@local` | Pre-seed admin email (only used if `admin.password` is set). |
| `admin.password` | `""` | Pre-seed admin password. Empty → create the admin on the first-visit setup screen. |

**Databases** (bundled TimescaleDB + Postgres; each gets a PVC)
| Key | Default | Purpose |
|---|---|---|
| `timescaledb.enabled` | `true` | `false` → use an external DB (set `hub.configDatabaseUrl`/`hub.dataDatabaseUrl`; the data DB **must** have TimescaleDB). |
| `timescaledb.image` | `timescale/timescaledb:2.17.2-pg16` | Data DB image (pinned — a moving tag can force a breaking PG major bump). |
| `timescaledb.configImage` | `postgres:16.6-alpine` | Config DB image (plain Postgres). |
| `timescaledb.password` | `""` | Leave empty — auto-generated and kept stable across `helm upgrade`. |
| `timescaledb.storageClass` | `""` | StorageClass for both DB PVCs (`""` = cluster default). |
| `timescaledb.configStorage` | `1Gi` | Config DB volume size (small). |
| `timescaledb.dataStorage` | `5Gi` | Data DB volume size (grows with metrics history). |
| `timescaledb.resources` | `100m/256Mi req, 1Gi limit` | Per-DB pod resources. |

**Hub pod**
| Key | Default | Purpose |
|---|---|---|
| `hub.replicas` | `1` | Hub replica count. |
| `hub.appSecret` | `""` | `EXEC_APP_SECRET` verbatim; empty + `autoAppSecret=true` auto-generates one. |
| `hub.autoAppSecret` | `true` | Auto-generate + persist the app secret in the release Secret. **Back it up** — losing it makes stored SSH keys unrecoverable. |
| `hub.configDatabaseUrl` | `""` | External config DB URL (only when `timescaledb.enabled=false`). |
| `hub.dataDatabaseUrl` | `""` | External data DB URL (TimescaleDB required). |
| `hub.resources` | `100m/128Mi req, 512Mi limit` | Hub pod resources. |
| `hub.securityContext` | non-root uid 10001, drop ALL, add `NET_RAW` | `NET_RAW` is needed for ICMP ping monitors. |
| `hub.nodeSelector` | `kubernetes.io/arch: amd64` | Hub image is amd64-only — keep it off arm64 nodes. |
| `hub.service.type` | `ClusterIP` | `NodePort`/`LoadBalancer` to expose the UI without an Ingress. |
| `hub.ingress.host` | `""` | Set a hostname → creates an Ingress (enables it implicitly). |
| `hub.ingress.className` | `nginx` | Ingress controller class (e.g. `traefik`). |
| `hub.ingress.tls` | `false` | `true` → terminate HTTPS via the `<release>-tls` secret (also sets passkey/`PUBLIC_URL` scheme to https). |
| `hub.ingress.annotations` | `{}` | e.g. `cert-manager.io/cluster-issuer: letsencrypt`. |
| `hub.ingress.enabled` | `false` | Usually left implied by `hub.ingress.host`. |

**Production hardening** (opt-in)
| Key | Default | Purpose |
|---|---|---|
| `backup.enabled` | `false` | Nightly `pg_dump` of both DBs → a PVC. |
| `backup.schedule` | `0 3 * * *` | Backup CronJob schedule. |
| `backup.keepDays` | `7` | Days of dumps to retain. |
| `backup.storage` | `5Gi` | Backup PVC size. |
| `backup.storageClass` | `""` | Backup PVC StorageClass. |
| `networkPolicy.enabled` | `false` | Default-deny + hub→DB allow (only meaningful with bundled DBs). |
| `podDisruptionBudget.enabled` | `false` | PDB for the hub (relevant with `hub.replicas>1`). |
| `podDisruptionBudget.minAvailable` | `1` | Minimum available hub pods during disruptions. |

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

**Updates** are driven externally — there's no in-cluster self-updater. Bump `image.tag`
(or point it at a moving tag like `:latest` / `:main` with `--set image.pullPolicy=Always`)
and `helm upgrade`, or let your GitOps tool roll it. The hub's `RollingUpdate` strategy
keeps redeploys gap-free even at `replicas: 1`.

### Expose the UI
- Domain (nginx ingress by default): `--set hub.ingress.host=monitor.example.com` (host alone enables it)
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
kubectl apply -f "https://monitor.example.com/pub/agent.yaml?key=<api-key>&cluster=k8s-hanoi"
```
The hub fills in its own URL, the key, and the cluster — no clone, no chart registry.
Defaults to the `:latest` image; add `&tag=main` (rolling) or `&tag=3.0.0` (pinned) to
choose another. To update, re-apply with the tag you want (or run `:latest`/`:main` with
`imagePullPolicy: Always`, already set in the served manifest, and restart the pods).

The served manifest installs BOTH the per-node DaemonSet (host metrics) and the
one-per-cluster collector (namespace / deployment / pod / **per-container CPU-RAM** —
the **Clusters** page). Container CPU/RAM needs **metrics-server** in the cluster;
without it the metadata still populates and usage reads 0.

**Helm** (if you prefer a release you manage) — the agent chart installs the DaemonSet
**and** the cluster-agent (Deployment + read-only ClusterRole). Same public OCI registry,
no clone needed:
```bash
helm install vantage-agent oci://ghcr.io/the-last-devops/charts/vantage-agent --version 3.0.0 \
  --namespace vantage --create-namespace \
  --set hubUrl=https://monitor.example.com \
  --set apiKey=<api-key-from-Add-System> \
  --set cluster=k8s-hanoi
# same cluster as the hub? use the in-cluster Service: --set hubUrl=http://vantage-hub.vantage:8080
# opt into the reverse SSH tunnel (off by default): --set allowShell=true
# per-node host metrics only (skip the cluster collector): --set clusterAgent.enabled=false
# from a checkout instead of the registry: helm install vantage-agent ./deploy/agent …
```

#### Agent chart values (`deploy/agent`)

| Key | Required | Default | Purpose |
|---|:---:|---|---|
| `hubUrl` | ✅ | `""` | Where the hub is reachable **from inside this cluster**. Same cluster as hub → `http://<release>-hub.<ns>:8080`; else the hub's public URL. |
| `apiKey` | ✅ | `""` | Enrollment key from the UI (**Add System**). |
| `cluster` | — | `my-cluster` | Label grouping this cluster's nodes in the UI (**Kubernetes › \<cluster\>**). |
| `image.repository` | — | `ghcr.io/the-last-devops/vantage-agent` | Agent image repository. |
| `image.tag` | — | `3.0.0` | Pinned tag; bump + `helm upgrade` (or a moving tag + `pullPolicy=Always`) to update. |
| `pullPolicy` | — | `IfNotPresent` | Set `Always` when tracking a moving tag. |
| `pullSecrets` | — | `[]` | imagePullSecrets for private GHCR, e.g. `{ghcr}`. |
| `clusterAgent.enabled` | — | `true` | Also deploy the one-per-cluster collector (Deployment + read-only ClusterRole) for the **Clusters** page. `false` = per-node host metrics only. |
| `allowShell` | — | `false` | Open the reverse SSH tunnel so the hub can console into nodes (opt-in; still gated by RBAC + step-up + host SSH auth). See `docs/exec-design.md`. |
| `resources` | — | `20m/32Mi req, 128Mi limit` | Agent pod resources. |

#### Agent environment variables (Docker / binary / served manifest)

The DaemonSet/Compose/binary agent reads these directly:

| Variable | Required | Default | Purpose |
|---|:---:|---|---|
| `HUB_URL` | ✅ | — | Base URL of the hub the agent pushes to. |
| `API_KEY` | ✅ | — | Per-server enrollment key (sent as `x-api-key`); the hub maps it to a workspace. |
| `AGENT_KIND` | — | auto-detect | Force `node` / `docker` / `k8s` / `k8s-cluster` instead of auto-detecting. |
| `CLUSTER` | — | — | Cluster label for grouping (Kubernetes). |
| `ALLOW_SHELL` | — | `0` (charts) | `1` opens the reverse SSH tunnel (see `allowShell`). |
| `INTERVAL` | — | hub-controlled | Optional push-cadence override (seconds); normally the hub decides. |
| `HOSTNAME_OVERRIDE` | — | system hostname | Name this host reports as. |
| `DISK_PATH` | — | `/` | Filesystem to report disk usage for (`/host` when the host root is mounted into a container). |
| `NODE_NAME` | — | (k8s downward API) | Node name when running as a DaemonSet. |

For a single host outside k8s: `curl -fsSL https://monitor.example.com/pub/install.sh | HUB_URL=… API_KEY=… sh`
(native binary + systemd), or run the agent container directly.

## Images & charts
`ghcr.io/the-last-devops/vantage-{hub,agent}` — tagged releases publish `:<version>`
(e.g. `:3.0.0`) + `:latest`; `:main` is the rolling build from `main`. The chart pins
`:3.0.0` by default.

Helm **charts** ship the same way — public OCI artifacts published per release tag:
`oci://ghcr.io/the-last-devops/charts/vantage` (hub) and `…/vantage-agent` (agent), each
at the release version. That's why the install commands above need no `git clone`.

If the packages are private:
```bash
kubectl -n vantage create secret docker-registry ghcr \
  --docker-server=ghcr.io --docker-username=<gh-user> --docker-password=<PAT read:packages>
helm ... --set image.pullSecrets='{ghcr}'   # agent chart: same flag
```
