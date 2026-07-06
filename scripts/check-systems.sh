#!/usr/bin/env bash
# Verify end-to-end: login + GET /api/systems, then summarize by kind/cluster.
set -u
HUB_URL="${HUB_URL:-http://localhost:8080}"
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@local}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-admin123}"

node - "$HUB_URL" "$ADMIN_EMAIL" "$ADMIN_PASSWORD" <<'EOF'
const [HUB, email, password] = process.argv.slice(2);
let cookie = '';
async function api(m, p, b) {
  const h = {}; if (cookie) h.cookie = cookie; if (b) h['content-type'] = 'application/json';
  const r = await fetch(HUB + p, { method: m, headers: h, body: b && JSON.stringify(b) });
  const sc = r.headers.get('set-cookie'); if (sc) cookie = sc.split(';')[0];
  if (!r.ok) throw new Error(`${m} ${p} -> ${r.status}`);
  return r.headers.get('content-type')?.includes('json') ? r.json() : r.text();
}
(async () => {
  await api('POST', '/api/auth/login', { email, password });
  const servers = await api('GET', '/api/systems');
  const by = {}; const clusters = {};
  for (const s of servers) {
    by[s.kind] = (by[s.kind] || 0) + 1;
    if (s.cluster) clusters[s.cluster] = (clusters[s.cluster] || 0) + 1;
  }
  console.log('total servers:', servers.length);
  console.log('by kind:', JSON.stringify(by));
  console.log('k8s clusters:', JSON.stringify(clusters));
  console.log('sample:', servers.slice(0, 3).map(s => `${s.name}[${s.kind}/ws:${s.workspace}] cpu=${Math.round(s.cpu_percent||0)} agent=${s.agent_version || '—'}`).join('  '));
})().catch(e => { console.error(e.message); process.exit(1); });
EOF
