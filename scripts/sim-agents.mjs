#!/usr/bin/env node
// Test-agent simulator: enrolls many fake hosts (node / docker / k8s) and pushes
// realistic MetricsReport to the hub's /api/ingest on an interval — so the
// dashboard + metrics pipeline have live, sizable data to test against.
//
// Backend reality (see crates/hub): servers are flat, auto-registered by
// (token, hostname). There is no node/docker/k8s "kind" column yet, so we model:
//   - node   → host metrics only
//   - docker → host metrics + containers[]
//   - k8s    → one server per node, named "<cluster>-<role>-N" (no cluster entity yet)
//
// Usage:
//   HUB_URL=http://localhost:8080 ADMIN_EMAIL=a@b.c ADMIN_PASSWORD=secret \
//     node scripts/sim-agents.mjs
//
// Env (all optional except admin creds):
//   HUB_URL          default http://localhost:8080
//   ADMIN_EMAIL/ADMIN_PASSWORD   admin login (to create namespaces + tokens)
//   INTERVAL         seconds between pushes (default 5)
//   NODES            standalone nodes (default 20)
//   DOCKER           docker hosts (default 6)
//   CONTAINERS       containers per docker host (default 8)
//   K8S_CLUSTERS     k8s clusters (default 2)
//   K8S_NODES        nodes per cluster (default 5)
//   DURATION         stop after N seconds (default 0 = run forever)

import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
const CACHE = join(dirname(fileURLToPath(import.meta.url)), '.sim-tokens.json');

const HUB = process.env.HUB_URL || 'http://localhost:8080';
const ADMIN_EMAIL = process.env.ADMIN_EMAIL;
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD;
const INTERVAL = (parseInt(process.env.INTERVAL) || 5) * 1000;
const N_NODES = parseInt(process.env.NODES ?? '20');
const N_DOCKER = parseInt(process.env.DOCKER ?? '6');
const N_CONTAINERS = parseInt(process.env.CONTAINERS ?? '8');
const N_K8S_CLUSTERS = parseInt(process.env.K8S_CLUSTERS ?? '2');
const N_K8S_NODES = parseInt(process.env.K8S_NODES ?? '5');
const DURATION = (parseInt(process.env.DURATION) || 0) * 1000;

if (!ADMIN_EMAIL || !ADMIN_PASSWORD) {
  console.error('Need ADMIN_EMAIL and ADMIN_PASSWORD to create tokens.');
  process.exit(1);
}

const rnd = (a, b) => a + Math.random() * (b - a);
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const GIB = 1024 ** 3;

// --- HTTP helpers (manual cookie jar for the session) ---
let cookie = '';
async function api(method, path, body) {
  const headers = {};
  if (cookie) headers.cookie = cookie;
  if (body !== undefined) headers['content-type'] = 'application/json';
  const res = await fetch(HUB + path, { method, headers, body: body && JSON.stringify(body) });
  const setC = res.headers.get('set-cookie');
  if (setC) cookie = setC.split(';')[0];
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${await res.text()}`);
  const ct = res.headers.get('content-type') || '';
  return ct.includes('json') ? res.json() : res.text();
}

async function login() {
  await api('POST', '/api/auth/login', { email: ADMIN_EMAIL, password: ADMIN_PASSWORD });
  console.log('logged in as', ADMIN_EMAIL);
}

async function ensureNamespace(name) {
  const list = await api('GET', '/api/namespaces').catch(() => []);
  const found = (Array.isArray(list) ? list : list.namespaces || []).find((n) => n.name === name);
  if (found) return found.id;
  const created = await api('POST', '/api/namespaces', { name });
  return created.id || created;
}

async function createKey(nsId, name) {
  const k = await api('POST', `/api/namespaces/${nsId}/keys`, { name });
  return k.key;
}

// --- fleet model ---
function makeHost({ hostname, kind, token, cores, memGiB, nContainers = 0, cluster = '' }) {
  return {
    hostname, kind, token, cluster,
    cores, memTotal: memGiB * GIB,
    diskTotal: rnd(200, 2000) * GIB,
    // wandering gauges (percent)
    cpu: rnd(10, 50), mem: rnd(30, 70), diskPct: rnd(20, 80),
    // cumulative counters (bytes)
    netRx: 0, netTx: 0, diskRead: 0, diskWrite: 0,
    bootTs: Math.floor(Date.now() / 1000) - Math.floor(rnd(3600, 60 * 86400)),
    cpuModel: ['EPYC 7543', 'Xeon E-2388G', 'Ryzen 9 7950X', 'Graviton3'][Math.floor(rnd(0, 4))],
    kernel: '6.8.0-40-generic',
    hasTemp: kind !== 'k8s' && Math.random() < 0.6, // some hosts expose sensors
    containers: Array.from({ length: nContainers }, (_, i) => ({
      name: ['nginx', 'api', 'postgres', 'redis', 'worker', 'rabbitmq', 'minio', 'cron', 'proxy', 'cache'][i % 10] + (i >= 10 ? i : ''),
      cpu: rnd(2, 30), mem: rnd(0.1, 2) * GIB, netRx: 0, netTx: 0,
    })),
  };
}

function step(v, lo, hi, vol) { return clamp(v + rnd(-vol, vol), lo, hi); }

function report(h) {
  h.cpu = step(h.cpu, 2, 99, 8);
  h.mem = step(h.mem, 15, 96, 4);
  h.diskPct = step(h.diskPct, 5, 98, 0.5);
  const rxRate = rnd(0.2, 8) * 1e6, txRate = rnd(0.1, 5) * 1e6;
  h.netRx += rxRate * (INTERVAL / 1000); h.netTx += txRate * (INTERVAL / 1000);
  h.diskRead += rnd(0, 50) * 1e6; h.diskWrite += rnd(0, 30) * 1e6;
  for (const c of h.containers) {
    c.cpu = step(c.cpu, 0, 95, 6); c.mem = clamp(c.mem + rnd(-5e7, 5e7), 5e7, 4 * GIB);
    c.netRx += rnd(0, 2) * 1e6; c.netTx += rnd(0, 1.5) * 1e6;
  }
  return {
    ts: Math.floor(Date.now() / 1000),
    hostname: h.hostname,
    kind: h.kind,
    cluster: h.kind === 'k8s' ? h.cluster : '',
    cpu_percent: +h.cpu.toFixed(1),
    mem_used: Math.round((h.mem / 100) * h.memTotal),
    mem_total: h.memTotal,
    swap_used: Math.round(rnd(0, 0.2) * GIB),
    swap_total: 4 * GIB,
    disk_used: Math.round((h.diskPct / 100) * h.diskTotal),
    disk_total: Math.round(h.diskTotal),
    net_rx: Math.round(h.netRx),
    net_tx: Math.round(h.netTx),
    load1: +(h.cpu / 100 * h.cores * rnd(0.6, 1.2)).toFixed(2),
    uptime: Math.floor(Date.now() / 1000) - h.bootTs,
    agent_version: Math.random() < 0.15 ? '0.6.9' : Math.random() < 0.3 ? '0.7.0' : '0.7.1',
    kernel: h.kernel,
    cpu_model: h.cpuModel,
    cpu_cores: h.cores,
    disk_read: Math.round(h.diskRead),
    disk_write: Math.round(h.diskWrite),
    temps: h.hasTemp ? [
      { label: 'CPU', celsius: +step(50 + h.cpu * 0.3, 35, 92, 3).toFixed(1) },
      { label: 'NVMe', celsius: +rnd(38, 55).toFixed(1) },
    ] : [],
    containers: h.containers.map((c) => ({
      name: c.name, cpu_percent: +c.cpu.toFixed(1),
      mem_used: Math.round(c.mem), net_rx: Math.round(c.netRx), net_tx: Math.round(c.netTx),
    })),
    gpus: [],
  };
}

async function push(h) {
  try {
    await fetch(HUB + '/api/ingest', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-api-key': h.token },
      body: JSON.stringify(report(h)),
    });
  } catch (e) { /* transient; ignore */ }
}

// Reuse cached tokens across runs so re-running updates the SAME servers
// (token+hostname is the server identity) instead of creating duplicates.
async function getTokens() {
  if (existsSync(CACHE)) {
    try {
      const c = JSON.parse(readFileSync(CACHE, 'utf8'));
      if (c.prod && c.stg && c.edge) { console.log('reusing cached sim tokens'); return c; }
    } catch { /* fall through to recreate */ }
  }
  await login();
  const c = {
    prod: await createKey(await ensureNamespace('production'), 'simulator'),
    stg: await createKey(await ensureNamespace('staging'), 'simulator'),
    edge: await createKey(await ensureNamespace('edge'), 'simulator'),
  };
  writeFileSync(CACHE, JSON.stringify(c, null, 2));
  console.log('created sim tokens →', CACHE);
  return c;
}

async function main() {
  const { prod: tokProd, stg: tokStg, edge: tokEdge } = await getTokens();
  const nsTokens = [tokProd, tokProd, tokStg, tokEdge]; // weight prod higher
  const pickTok = () => nsTokens[Math.floor(rnd(0, nsTokens.length))];

  const fleet = [];
  for (let i = 1; i <= N_NODES; i++)
    fleet.push(makeHost({ hostname: `node-${String(i).padStart(2, '0')}`, kind: 'node', token: pickTok(), cores: [4, 8, 16, 32][i % 4], memGiB: [8, 16, 32, 64][i % 4] }));
  for (let i = 1; i <= N_DOCKER; i++)
    fleet.push(makeHost({ hostname: `docker-${String(i).padStart(2, '0')}`, kind: 'docker', token: pickTok(), cores: 8, memGiB: 32, nContainers: N_CONTAINERS }));
  for (let c = 1; c <= N_K8S_CLUSTERS; c++) {
    const tok = c === 1 ? tokProd : tokStg;
    const cluster = `cluster-${c}`;
    for (let n = 1; n <= N_K8S_NODES; n++)
      fleet.push(makeHost({ hostname: `k8s${c}-${n === 1 ? 'cp' : 'worker'}-${n}`, kind: 'k8s', token: tok, cores: 16, memGiB: 64, cluster }));
  }

  console.log(`fleet: ${fleet.length} hosts (${N_NODES} node, ${N_DOCKER} docker×${N_CONTAINERS} containers, ${N_K8S_CLUSTERS} k8s×${N_K8S_NODES} nodes)`);
  console.log(`pushing every ${INTERVAL / 1000}s to ${HUB}/api/ingest`);

  let ticks = 0;
  const tick = async () => {
    await Promise.all(fleet.map(push));
    if (++ticks % 6 === 0) console.log(`tick ${ticks} — ${fleet.length} hosts pushed`);
  };
  await tick();
  const timer = setInterval(tick, INTERVAL);
  if (DURATION > 0) setTimeout(() => { clearInterval(timer); console.log('done'); process.exit(0); }, DURATION);
}

main().catch((e) => { console.error(e.message); process.exit(1); });
