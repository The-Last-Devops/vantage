#!/usr/bin/env bash
# Verify the k8s cluster-stats ingest end-to-end: login, ensure a workspace + API
# key, POST a sample KubeReport to /pub/kube, then confirm the cluster shows up as a
# 'k8s-cluster' system. Idempotent + self-cleaning (removes its own 'smoke-kube' key).
set -u
HUB_URL="${HUB_URL:-http://localhost:8080}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"

node - "$HUB_URL" "$ADMIN_EMAIL" "$ADMIN_PASSWORD" <<'EOF'
const [HUB, email, password] = process.argv.slice(2);
let cookie = '';
async function api(m, p, b, key) {
  const h = {}; if (cookie) h.cookie = cookie;
  if (b) h['content-type'] = 'application/json';
  if (key) h['x-api-key'] = key;
  const r = await fetch(HUB + p, { method: m, headers: h, body: b && JSON.stringify(b) });
  const sc = r.headers.get('set-cookie'); if (sc) cookie = sc.split(';')[0];
  if (!r.ok) throw new Error(`${m} ${p} -> ${r.status}`);
  return r.status === 204 ? null : (r.headers.get('content-type')?.includes('json') ? r.json() : r.text());
}
const CLUSTER = 'smoke-cluster';
(async () => {
  await api('POST', '/api/auth/login', { email, password });

  // Workspace: reuse the first, else create one.
  let list = await api('GET', '/api/workspaces');
  let wss = Array.isArray(list) ? list : list.workspaces || [];
  let ws = wss[0] || await api('POST', '/api/workspaces', { name: 'smoke' });
  console.log('workspace:', ws.name, ws.id);

  // Fresh API key for this run (delete any leftover first).
  const keys = await api('GET', `/api/workspaces/${ws.id}/keys`).catch(() => []);
  for (const k of (Array.isArray(keys) ? keys : [])) if (k.name === 'smoke-kube') await api('DELETE', `/api/keys/${k.id}`).catch(()=>{});
  const created = await api('POST', `/api/workspaces/${ws.id}/keys`, { name: 'smoke-kube' });
  const key = created.key || created.api_key || created.token;
  if (!key) throw new Error('no api key in create response: ' + JSON.stringify(created));

  // Post a representative KubeReport.
  const report = {
    ts: Math.floor(Date.now() / 1000),
    cluster: CLUSTER,
    agent_version: 'smoke',
    namespaces: [
      { name: 'default', phase: 'Active', pods_total: 3, pods_running: 2, pods_pending: 1, pods_failed: 0, pods_succeeded: 0, restarts: 5 },
      { name: 'kube-system', phase: 'Active', pods_total: 8, pods_running: 8, pods_pending: 0, pods_failed: 0, pods_succeeded: 0, restarts: 0 },
    ],
    deployments: [
      { namespace: 'default', name: 'web', desired: 3, ready: 2, available: 2, updated: 3 },
      { namespace: 'default', name: 'api', desired: 1, ready: 1, available: 1, updated: 1 },
    ],
  };
  const ack = await api('POST', '/pub/kube', report, key);
  console.log('ingest ack:', JSON.stringify(ack));

  // The cluster should now be registered as a k8s-cluster system.
  const systems = await api('GET', '/api/systems');
  const cl = systems.find((s) => s.hostname === CLUSTER && s.kind === 'k8s-cluster');
  if (!cl) throw new Error(`cluster '${CLUSTER}' not registered as k8s-cluster system`);
  console.log('OK cluster system:', cl.name, `kind=${cl.kind}`, `cluster=${cl.cluster}`);

  // Clean up the smoke key (cascades the system + its rows).
  const k2 = await api('GET', `/api/workspaces/${ws.id}/keys`).catch(() => []);
  for (const k of (Array.isArray(k2) ? k2 : [])) if (k.name === 'smoke-kube') await api('DELETE', `/api/keys/${k.id}`).catch(()=>{});
  console.log('cleaned up smoke-kube key');
})().catch((e) => { console.error('FAIL:', e.message); process.exit(1); });
EOF
