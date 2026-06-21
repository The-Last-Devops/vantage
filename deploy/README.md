# Deploying last-monitor

## Local dev
`docker compose up -d` (hub + TimescaleDB + a bundled agent). UI at http://localhost:8080.

## Kubernetes (Helm) — hub + DB + agent in one cluster
The chart in `deploy/chart` installs the hub, an in-cluster TimescaleDB, and the
agent DaemonSet (one pod per node). The agent talks to the hub over the in-cluster
Service, so **no public hub URL / tunnel is needed** for the cluster to report.

```bash
helm install lm ./deploy/chart \
  --namespace last-monitor --create-namespace \
  --set admin.password='a-strong-password' \
  --set apiKey="$(openssl rand -hex 16)" \
  --set cluster=prod-cluster
```

What you get:
- TimescaleDB (StatefulSet + PVC), hub (Deployment + Service), agent (DaemonSet).
- The hub bootstraps the `apiKey` into the `default` namespace, so the agent
  auto-enrolls with no UI step. Nodes appear under **Kubernetes › <cluster>**.

### Prerequisites
- **Images** `ghcr.io/the-last-devops/last-monitor-{hub,agent}:main` must be pullable.
  GHCR packages are private by default — either make them **public**, or:
  ```bash
  kubectl -n last-monitor create secret docker-registry ghcr \
    --docker-server=ghcr.io --docker-username=<gh-user> --docker-password=<PAT read:packages>
  helm ... --set image.pullSecrets='{ghcr}'
  ```

### View the UI
- Quick: `kubectl -n last-monitor port-forward svc/lm-hub 8080:8080` → http://localhost:8080
- Or set `--set hub.service.type=NodePort`, or enable ingress:
  `--set hub.ingress.enabled=true --set hub.ingress.host=mon.example.com --set hub.ingress.className=nginx`

### External DB instead of in-cluster
`--set timescaledb.enabled=false --set hub.configDatabaseUrl=... --set hub.dataDatabaseUrl=...`

## Agent only (existing hub) — standalone manifest
If the hub already runs elsewhere, skip Helm and use `deploy/k8s/agent.yaml`
(edit HUB_URL / API_KEY / CLUSTER / IMAGE, then `kubectl apply -f`).
