#!/usr/bin/env bash
# Xoá sạch token + server do simulator tạo (tên token 'simulator' hoặc 'sim-*').
# Xoá token sẽ cascade xoá server gắn token đó. Cũng xoá cache token cục bộ.
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
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
  const ct = r.headers.get('content-type') || '';
  return r.status === 204 ? null : (ct.includes('json') ? r.json() : r.text());
}
const isSim = (n) => n === 'simulator' || n.startsWith('sim-');
(async () => {
  await api('POST', '/api/auth/login', { email, password });
  const nss = await api('GET', '/api/namespaces');
  let deleted = 0;
  for (const ns of (Array.isArray(nss) ? nss : nss.namespaces || [])) {
    const keys = await api('GET', `/api/namespaces/${ns.id}/keys`).catch(() => []);
    for (const k of (Array.isArray(keys) ? keys : [])) {
      if (isSim(k.name)) { await api('DELETE', `/api/keys/${k.id}`).catch(() => {}); deleted++; }
    }
  }
  console.log(`deleted ${deleted} simulator key(s) (systems cascade-deleted)`);
})().catch(e => { console.error(e.message); process.exit(1); });
EOF

rm -f "$REPO/scripts/.sim-tokens.json" && echo "cleared local token cache"
