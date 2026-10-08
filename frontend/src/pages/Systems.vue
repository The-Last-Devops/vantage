<script setup>
import { ref, reactive, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import Gauge from '../components/Gauge.vue'
import { useCached } from '../lib/cache'
import AddSystemModal from '../components/AddSystemModal.vue'
import SystemSearch from '../components/SystemSearch.vue'
import Sparkline from '../components/Sparkline.vue'
import { insertGaps } from '../lib/gaps'
import { pct, online, parseQuery, matchPred } from '../lib/hostFilter'
import { DEFAULT_THR, ago } from '../lib/triage'

const showAdd = ref(false)

const route = useRoute()
const router = useRouter()
const servers = ref([])
const error = ref('')
const q = ref(route.query.q || '')
let qTimer
watch(q, (v) => { clearTimeout(qTimer); qTimer = setTimeout(() => router.replace({ query: { ...route.query, q: v || undefined } }), 300) })
// keep q in sync with the URL too, so navigating to a clean "/" (e.g. clicking
// "Systems") actually clears the chips instead of the stale ref re-adding them
watch(() => route.query.q, (v) => { if ((v || '') !== q.value) q.value = v || '' })
// workspace filter from URL (?ws=a,b ; empty = all) — shared/persisted, set in the sidebar
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const inWs = (s) => selectedWs.value.length === 0 || selectedWs.value.includes(s.workspace)
const selected = reactive(new Set())
const expanded = reactive(new Set())
const containers = reactive({}) // dockerSystemId -> [{name, cpu, mem}]
const lastNonNull = (d) => { if (!d) return null; for (let i = d.length - 1; i >= 0; i--) if (d[i] != null) return d[i]; return null }
async function toggleDocker(s) {
  if (expanded.has(s.id)) { expanded.delete(s.id); return }
  expanded.add(s.id)
  if (!containers[s.id]) {
    try {
      const h = await api.get(`/api/systems/${s.id}/containers`)
      const memBy = Object.fromEntries((h.mem || []).map((x) => [x.name, x]))
      containers[s.id] = (h.cpu || []).map((x) => ({ name: x.name, cpu: Math.round(lastNonNull(x.data) ?? 0), mem: lastNonNull(memBy[x.name]?.data) }))
    } catch { containers[s.id] = [] }
  }
}
const sortState = reactive({ col: 'name', dir: 'asc' })
const KIND_LABEL = { node: 'Node', docker: 'Docker', k8s: 'K8s', 'k8s-cluster': 'Cluster' }
let timer = null

const r = (x) => Math.round(x || 0)
const LATEST = computed(() => servers.value.map((s) => s.agent_version).filter(Boolean).sort(cmpVer).pop())
function cmpVer(a, b) { const p = (x) => x.split('.').map(Number); const A = p(a), B = p(b); for (let i = 0; i < 3; i++) if ((A[i]||0)!==(B[i]||0)) return (A[i]||0)-(B[i]||0); return 0 }
function agentCls(v) { if (!v) return 'bg-surface2 text-faint'; if (v === LATEST.value) return 'bg-accent/10 text-accent'; return cmpVer(v, '0.7.0') >= 0 ? 'bg-warn/10 text-warn' : 'bg-down/10 text-down' }

// Host search mini-language (parseQuery/matchPred) + pct/online → ../lib/hostFilter.
// committed filters shown as chips (each token in q); search box appends via @add
const chips = computed(() => q.value.trim().split(/\s+/).filter(Boolean))
function addToken(tok) { const t = (tok || '').trim(); if (t) q.value = q.value.trim() ? `${q.value.trim()} ${t}` : t }
function removeChip(i) { const a = chips.value.slice(); a.splice(i, 1); q.value = a.join(' ') }
// reset clears both the text filters (?q) and the pinned-node selection (?fsel)
function resetFilters() { q.value = ''; selected.clear(); router.replace({ query: { ...route.query, q: undefined } }) }
const preds = computed(() => parseQuery(q.value))
// "Needs attention" sub-view (/attention) narrows everything to abnormal hosts.
const attnMode = computed(() => route.name === 'attention')
// Optional ?status= focuses the sub-view on one severity (down/crit/warn).
const SEV_OF_STATUS = { down: 3, crit: 2, warn: 1 }
const attnStatus = computed(() => SEV_OF_STATUS[route.query.status] ?? null)
// Page heading/title: reflect the focused status when one is set, else "Issues".
const ATTN_LABEL = { down: 'Down', crit: 'Critical', warn: 'Warning' }
const attnTitle = computed(() => (attnMode.value ? ATTN_LABEL[route.query.status] || 'Issues' : 'Infrastructure'))
const visible = computed(() => {
  let list = servers.value.filter((s) => inWs(s) && preds.value.every((p) => matchPred(s, p)))
  if (attnMode.value) {
    list = attnStatus.value != null ? list.filter((s) => sevOf(s) === attnStatus.value) : list.filter((s) => sevOf(s) > 0)
  }
  return list
})
function sortList(list, st) {
  const f = {
    name: (a, b) => a.name.localeCompare(b.name),
    type: (a, b) => (a.kind || '').localeCompare(b.kind || '') || a.name.localeCompare(b.name),
    cluster: (a, b) => (a.cluster || '').localeCompare(b.cluster || '') || a.name.localeCompare(b.name),
    ws: (a, b) => (a.workspace || '').localeCompare(b.workspace || ''),
    status: (a, b) => Number(online(b)) - Number(online(a)),
    cpu: (a, b) => (a.cpu_percent || 0) - (b.cpu_percent || 0),
    mem: (a, b) => (pct(a.mem_used, a.mem_total) || 0) - (pct(b.mem_used, b.mem_total) || 0),
    disk: (a, b) => (pct(a.disk_used, a.disk_total) || 0) - (pct(b.disk_used, b.disk_total) || 0),
    agent: (a, b) => (a.agent_version || '').localeCompare(b.agent_version || ''),
  }[st.col]
  const out = [...list].sort(f || (() => 0))
  return st.dir === 'desc' ? out.reverse() : out
}
// one flat host list (node / docker / k8s); type & cluster are row attributes
const rows = computed(() => sortList(visible.value, sortState))
function avg(arr, f) { const v = arr.map(f).filter((x) => x != null); return v.length ? Math.round(v.reduce((a, b) => a + b, 0) / v.length) : null }
// Avg CPU / memory / disk follow the range picker (issue #3): the mean of every bucket
// of every visible host over that window, from the same /api/fleet series the row
// sparklines draw. Before the fleet data lands, fall back to the latest sample so the
// tiles never sit empty. "Systems online" is a count, not an average — it stays "now".
function avgSeries(list) {
  let sum = 0, cnt = 0
  for (const s of list) for (const v of s.data) if (v != null) { sum += v; cnt++ }
  return cnt ? Math.round(sum / cnt) : null
}
const hero = computed(() => {
  const all = visible.value, on = all.filter(online).length
  const f = fleet.value, names = new Set(all.map((s) => s.name))
  const over = (k) => (f && f[k] ? avgSeries(f[k].filter((s) => names.has(s.name))) : null)
  return {
    online: on, total: all.length,
    cpu: over('cpu') ?? avg(all, (x) => x.cpu_percent),
    mem: over('mem') ?? avg(all, (x) => pct(x.mem_used, x.mem_total)),
    disk: over('disk') ?? avg(all, (x) => pct(x.disk_used, x.disk_total)),
  }
})

// ---- thresholds + "needs attention" triage --------------------------------
const thresholds = ref({}) // workspace name -> thresholds object
async function loadThresholds() {
  try { const r = await api.get('/api/thresholds'); const m = {}; for (const x of r) m[x.workspace] = x; thresholds.value = m } catch {}
}
const thrOf = (s) => thresholds.value[s.workspace] || DEFAULT_THR
const metricsOf = (s) => ({ cpu: s.cpu_percent, mem: pct(s.mem_used, s.mem_total), disk: pct(s.disk_used, s.disk_total), dutil: s.disk_util })
// severity: 0 ok · 1 warn · 2 crit · 3 down
function sevOf(s) {
  if (!online(s)) return 3
  const t = thrOf(s), m = metricsOf(s)
  let lvl = 0
  const chk = (v, w, c) => { if (v == null) return; if (v >= c) lvl = Math.max(lvl, 2); else if (v >= w) lvl = Math.max(lvl, 1) }
  chk(m.cpu, t.cpu_warn, t.cpu_crit); chk(m.mem, t.mem_warn, t.mem_crit); chk(m.disk, t.disk_warn, t.disk_crit); chk(m.dutil, t.dutil_warn, t.dutil_crit)
  return lvl
}
// One flat list of abnormal hosts; each carries badges for what's wrong.
const ISSUE = {
  down: { label: 'Offline', icon: 'M18.36 6.64a9 9 0 1 1-12.73 0M12 2v10' },
  disk: { label: 'Disk space', icon: 'M22 12H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11ZM6 16h.01M10 16h.01' },
  cpu: { label: 'CPU', icon: 'M6 6h12v12H6zM9 1v3M15 1v3M9 20v3M15 20v3M20 9h3M20 14h3M1 9h3M1 14h3' },
  mem: { label: 'Memory', icon: 'M3 8h18v8H3zM7 8v8M12 8v8M17 8v8' },
  dutil: { label: 'Disk I/O', icon: 'M22 12h-4l-3 9L9 3l-3 9H2' },
}
const ISSUE_ORDER = ['down', 'disk', 'cpu', 'mem', 'dutil']
const attnHosts = computed(() => {
  const out = []
  for (const s of visible.value) {
    const issues = []
    if (!online(s)) {
      issues.push({ key: 'down', crit: true, val: null })
    } else {
      const t = thrOf(s), m = metricsOf(s)
      for (const k of ['disk', 'cpu', 'mem', 'dutil']) {
        const v = m[k], w = t[k + '_warn'], c = t[k + '_crit']
        if (v == null || v < w) continue
        issues.push({ key: k, crit: v >= c, val: Math.round(v) })
      }
    }
    if (!issues.length) continue
    issues.sort((a, b) => ISSUE_ORDER.indexOf(a.key) - ISSUE_ORDER.indexOf(b.key))
    out.push({ s, issues, crit: issues.some((i) => i.crit), top: Math.max(...issues.map((i) => i.val ?? 101)) })
  }
  return out.sort((a, b) => Number(b.crit) - Number(a.crit) || b.top - a.top)
})
// human-readable problem text for tooltips
const issueText = (i, s) => (i.key === 'down' ? `Offline for ${ago(s?.last_seen) || 'an unknown time'} — not reporting in` : `High ${ISSUE[i.key].label.toLowerCase()}: ${i.val}% (${i.crit ? 'critical' : 'warning'})`)
const chipTitle = (h) => `${h.s.name} · ${h.s.workspace}\n` + h.issues.map((i) => issueText(i, h.s)).join('\n')
// Picking a new column defaults to descending — we usually want the busiest
// (near-overload) hosts at the top; click again to flip to ascending.
function sortBy(col) { if (sortState.col === col) sortState.dir = sortState.dir === 'asc' ? 'desc' : 'asc'; else { sortState.col = col; sortState.dir = 'desc' } }
const arrow = (col) => (sortState.col === col ? (sortState.dir === 'desc' ? ' ↓' : ' ↑') : '')
// click a row attribute (type/cluster/ws) → set that filter dimension (replacing any existing)
function setFilter(key, val) { const toks = chips.value.filter((t) => !t.toLowerCase().startsWith(key + ':')); toks.push(`${key}:${val}`); q.value = toks.join(' ') }
function toggleRow(id) { selected.has(id) ? selected.delete(id) : selected.add(id) }
function toggleAll(rows) { const all = rows.length && rows.every((s) => selected.has(s.id)); rows.forEach((s) => (all ? selected.delete(s.id) : selected.add(s.id))) }
function toggleExpand(k) { expanded.has(k) ? expanded.delete(k) : expanded.add(k) }
async function bulkDelete() {
  const n = selected.size
  if (!n) return
  if (!(await confirm({ title: `Delete ${n} system${n > 1 ? 's' : ''}?`, message: `This removes ${n > 1 ? 'them' : 'it'} and all collected metrics. This cannot be undone.`, danger: true, confirmText: `Delete ${n}` }))) return
  for (const id of [...selected]) { try { await api.del(`/api/systems/${id}`) } catch {} }
  selected.clear(); await load()
}

// ---- Per-host history (issue #1): one /api/fleet call feeds a CPU and a memory
// sparkline in every row, replacing the four overlay charts that stacked 70 lines on
// one axis. 7d / 30d (issue #2) read the hourly tier the hub already serves.
const FRANGES = ['30m', '1h', '3h', '6h', '12h', '24h', '7d', '30d']
const frange = computed(() => (FRANGES.includes(route.query.frange) ? route.query.frange : '24h'))
function setFrange(r) { router.replace({ query: { ...route.query, frange: r } }) }
const fleet = ref(null)
async function loadFleet() { try { fleet.value = await api.get(`/api/fleet?range=${frange.value}`) } catch {} }
// stable host → color map (by sorted name) so the row dot and its sparkline match
const colorOf = computed(() => {
  const names = [...new Set(servers.value.map((s) => s.name))].sort()
  const m = {}
  names.forEach((n, i) => { m[n] = `hsl(${(i * 47) % 360} 70% 58%)` })
  return m
})
// fleet data with null breaks inserted at timeline gaps (agents stopped), keyed by host
const trend = computed(() => {
  const f = fleet.value
  if (!f || !f.t || f.t.length < 3) return { t: f?.t || [], by: {} }
  const arrays = [], map = []
  ;['cpu', 'mem'].forEach((g) => (f[g] || []).forEach((s) => { arrays.push(s.data); map.push([g, s.name]) }))
  const { t, arrays: na } = insertGaps(f.t, arrays)
  const by = {}
  map.forEach(([g, name], k) => { (by[name] ||= {})[g] = na[k] })
  return { t, by }
})
const trendOf = (s, g) => trend.value.by[s.name]?.[g] || []

// `/api/systems` is global (not workspace-scoped), so one cache key — navigating
// back to Systems paints the last fleet instantly, then revalidates silently.
const { loaded, reload: load } = useCached({
  key: () => 'systems',
  load: () => api.get('/api/systems'),
  // k8s clusters (kind 'k8s-cluster') are NOT hosts — they live on their own
  // Clusters page (/clusters), not in this fleet list.
  apply: (list) => { servers.value = list.filter((s) => s.kind !== 'k8s-cluster'); error.value = '' },
  // Keep showing existing data on a transient poll failure; only surface an
  // error before the first successful load.
  onError: () => { if (!servers.value.length) error.value = 'Failed to load systems' },
})
// The fleet series is one query over every host for the whole range; at 30d it is
// not something to refetch every 5 s like the row list. Once a minute is plenty for
// a strip 96 px wide.
let fleetTimer = null
onMounted(() => { load(); loadFleet(); loadThresholds(); timer = setInterval(load, 5000); fleetTimer = setInterval(loadFleet, 60000) })
onUnmounted(() => { clearInterval(timer); clearInterval(fleetTimer) })
watch(frange, loadFleet)

// A k8s NODE row IS a node → open its node detail (with cluster breadcrumb). A
// k8s-CLUSTER row is the cluster aggregate → its own Cluster page (namespace /
// workload / label breakdown, CPU/RAM, pod drill-down).
const detailLink = (s) => {
  const n = encodeURIComponent(s.name)
  if (s.kind === 'k8s-cluster') return `/cluster/${s.id}?name=${n}`
  if (s.kind === 'k8s') return `/system/${s.id}?type=node&name=${n}&parent=${encodeURIComponent(s.cluster || '')}&ptype=k8s`
  return `/system/${s.id}?type=${s.kind}&name=${n}`
}
</script>

<template>
  <AppShell :title="attnTitle">
    <div class="space-y-5">
      <!-- hero -->
      <section class="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="text-xs uppercase tracking-wider text-faint">Systems online <span class="normal-case tracking-normal">· now</span></div>
          <div class="mt-1.5 font-mono text-metric text-fg">{{ hero.online }}<span class="text-sm text-faint"> / {{ hero.total }}</span></div>
          <div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.total ? (hero.online / hero.total) * 100 : 0) + '%' }"></div></div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg disk <span class="normal-case tracking-normal">· {{ frange }}</span></div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.disk ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.disk || 0) + '%' }"></div></div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg CPU <span class="normal-case tracking-normal">· {{ frange }}</span></div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.cpu ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.cpu || 0) + '%' }"></div></div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg memory <span class="normal-case tracking-normal">· {{ frange }}</span></div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.mem ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.mem || 0) + '%' }"></div></div></div>
      </section>

      <!-- needs attention: a single compact list, icons show what's wrong -->
      <section v-if="attnMode && !attnHosts.length && loaded" class="rounded-xl border border-accent/30 bg-accent/5 p-6 text-center">
        <div class="flex items-center justify-center gap-2 text-sm font-medium text-accent">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M20 6 9 17l-5-5"/></svg>
          All systems healthy
        </div>
        <p class="mt-1 text-xs text-muted">No host is down or over its thresholds.</p>
      </section>
      <section v-if="attnMode && attnHosts.length" class="overflow-hidden rounded-xl border border-warn/30 bg-warn/5">
        <div class="flex items-center gap-2 px-4 py-3">
          <svg class="h-4 w-4 text-warn" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4M12 17h.01"/></svg>
          <h2 class="text-sm font-semibold text-fg">{{ attnTitle }}</h2>
          <span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ attnHosts.length }} hosts</span>
        </div>
        <div class="flex flex-wrap gap-2 border-t border-line/60 p-3">
          <RouterLink v-for="h in attnHosts" :key="h.s.id" :to="{ name: 'system', params: { id: h.s.id } }"
            v-tip="chipTitle(h)" class="inline-flex items-center gap-2 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-xs hover:border-accent/50">
            <span class="text-fg">{{ h.s.name }}</span>
            <span v-for="i in h.issues" :key="i.key" v-tip="issueText(i, h.s)"
              class="inline-flex items-center gap-0.5 font-mono tabular-nums" :class="i.crit ? 'text-down' : 'text-warn'">
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path :d="ISSUE[i.key].icon"/></svg>
              <span v-if="i.val != null">{{ i.val }}%</span><span v-else-if="ago(h.s.last_seen)">{{ ago(h.s.last_seen) }}</span>
            </span>
          </RouterLink>
        </div>
      </section>

      <!-- toolbar: search + add sit together on the left -->
      <div class="flex flex-wrap items-center gap-3">
        <SystemSearch :items="servers" @add="addToken" />
        <button @click="showAdd = true" class="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> Add system</button>
      </div>

      <PageLoader v-if="!loaded && !error" />
      <p v-if="error" class="text-sm text-down">{{ error }}</p>

      <!-- Hosts: one flat table; Type / Cluster / Workspace are clickable filters -->
      <section v-if="rows.length">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <h2 class="text-sm font-semibold text-fg">Hosts</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ rows.length }}</span>
          <!-- active filter chips (each token in the query) + reset -->
          <span v-for="(c, i) in chips" :key="c + i" class="flex items-center gap-1 rounded-full border border-line bg-surface2 py-0.5 pl-2 pr-1 text-xs text-fg">
            <span class="font-mono tabular-nums">{{ c }}</span>
            <button @click="removeChip(i)" v-tip="`Remove filter`" class="grid h-4 w-4 place-items-center rounded-full text-faint hover:bg-down/15 hover:text-down"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </span>
          <button v-if="chips.length" @click="resetFilters" class="text-xs text-muted hover:text-accent">Reset</button>
          <!-- range: drives the row sparklines and the Avg tiles above -->
          <div class="ml-auto flex rounded-lg border border-line bg-surface2 p-0.5 text-xs">
            <button v-for="rr in FRANGES" :key="rr" @click="setFrange(rr)" class="rounded-md px-2.5 py-1" :class="frange===rr?'bg-accent/15 font-medium text-accent':'text-muted hover:text-fg'">{{ rr }}</button>
          </div>
        </div>
        <div class="overflow-x-auto rounded-xl border border-line">
          <table class="w-full min-w-[1120px] text-sm">
            <thead class="border-b border-line2 bg-head text-left text-xs uppercase tracking-wide text-fg"><tr>
              <th class="w-8 px-3 py-2.5"><input type="checkbox" :checked="rows.length && rows.every((s)=>selected.has(s.id))" @change="toggleAll(rows)" class="h-4 w-4 accent-accent" /></th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('name')">Host{{ arrow('name') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('ws')">Workspace{{ arrow('ws') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('type')">Type{{ arrow('type') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('cluster')">Cluster{{ arrow('cluster') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('status')">Status{{ arrow('status') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('cpu')">CPU{{ arrow('cpu') }} <span class="font-normal normal-case tracking-normal text-faint">· {{ frange }}</span></th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('mem')">Memory{{ arrow('mem') }} <span class="font-normal normal-case tracking-normal text-faint">· {{ frange }}</span></th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('disk')">Disk{{ arrow('disk') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('agent')">Agent{{ arrow('agent') }}</th>
            </tr></thead>
            <tbody>
              <template v-for="s in rows" :key="s.id">
                <tr class="vantage-row border-b border-line border-l-2" :class="[selected.has(s.id) ? 'sel' : '', sevOf(s) === 3 || sevOf(s) === 2 ? 'border-l-down' : sevOf(s) === 1 ? 'border-l-warn' : 'border-l-transparent']">
                  <td class="px-3 py-3"><input type="checkbox" :checked="selected.has(s.id)" @change="toggleRow(s.id)" class="h-4 w-4 accent-accent" /></td>
                  <td class="px-4 py-3 whitespace-nowrap">
                    <div class="flex items-center gap-1.5">
                      <button v-if="s.kind === 'docker'" @click="toggleDocker(s)" class="text-muted hover:text-accent"><svg class="h-4 w-4 transition-transform" :class="expanded.has(s.id) ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></button>
                      <span v-else class="w-4 shrink-0"></span>
                      <span class="h-2 w-2 shrink-0 rounded-full" v-tip="online(s) ? 'online' : 'offline'" :style="{ background: colorOf[s.name] }"></span>
                      <RouterLink :to="detailLink(s)" class="font-mono text-fg hover:text-accent">{{ s.name }}</RouterLink>
                    </div>
                  </td>
                  <td class="whitespace-nowrap px-4 py-3"><button @click="setFilter('ws', s.workspace)" v-tip="`Filter ws:${s.workspace}`" class="whitespace-nowrap rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.workspace || '—' }}</button></td>
                  <td class="px-4 py-3"><button @click="setFilter('kind', s.kind)" v-tip="`Filter kind:${s.kind}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ KIND_LABEL[s.kind] || s.kind }}</button></td>
                  <td class="whitespace-nowrap px-4 py-3"><button v-if="s.cluster" @click="setFilter('cluster', s.cluster)" v-tip="`Filter cluster:${s.cluster}`" class="whitespace-nowrap rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.cluster }}</button><span v-else class="text-faint">—</span></td>
                  <td class="px-4 py-3 whitespace-nowrap"><button @click="setFilter('status', online(s)?'online':'offline')" v-tip="online(s) ? `Filter status:online` : `Last report ${s.last_seen ? new Date(s.last_seen).toLocaleString() : 'unknown'} · filter status:offline`" class="text-sm hover:underline" :class="online(s)?'text-accent':'text-down'">{{ online(s)?'online':'offline' }}<span v-if="!online(s) && ago(s.last_seen)" class="ml-1 font-mono text-xs tabular-nums text-down/80">{{ ago(s.last_seen) }}</span></button></td>
                  <td class="px-4 py-3"><div class="flex items-center gap-3"><Gauge :v="online(s)?r(s.cpu_percent):null" :warn="thrOf(s).cpu_warn" :crit="thrOf(s).cpu_crit" /><Sparkline :data="trendOf(s, 'cpu')" :time="trend.t" :max="100" :width="72" :color="colorOf[s.name]" /></div></td>
                  <td class="px-4 py-3"><div class="flex items-center gap-3"><Gauge :v="online(s)?pct(s.mem_used,s.mem_total):null" :warn="thrOf(s).mem_warn" :crit="thrOf(s).mem_crit" /><Sparkline :data="trendOf(s, 'mem')" :time="trend.t" :max="100" :width="72" :color="colorOf[s.name]" /></div></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?pct(s.disk_used,s.disk_total):null" :warn="thrOf(s).disk_warn" :crit="thrOf(s).disk_crit" /></td>
                  <td class="px-4 py-3"><span class="rounded px-1.5 py-0.5 text-xs" :class="agentCls(s.agent_version)">{{ s.agent_version ? 'v'+s.agent_version : '—' }}</span></td>
                </tr>
                <tr v-for="c in (containers[s.id] || [])" v-show="s.kind === 'docker' && expanded.has(s.id)" :key="s.id + ':' + c.name" class="vantage-row border-b border-line bg-bg/40">
                  <td></td>
                  <td class="px-4 py-2"><RouterLink :to="`/system/${s.id}?type=container&name=${encodeURIComponent(c.name)}&parent=${encodeURIComponent(s.name)}&ptype=docker`" class="flex items-center gap-2 pl-10 font-mono text-sm text-fg hover:text-accent"><span class="text-faint">└</span>{{ c.name }}</RouterLink></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2"><span class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-faint">container</span></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2 text-sm text-accent">running</td>
                  <td class="px-4 py-2"><Gauge :v="c.cpu" :warn="thrOf(s).cpu_warn" :crit="thrOf(s).cpu_crit" /></td>
                  <td class="px-4 py-2 font-mono tabular-nums text-muted">{{ c.mem != null ? (c.mem / 1048576).toFixed(0) + ' MB' : '—' }}</td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2 text-faint">—</td>
                </tr>
              </template>
            </tbody>
          </table>
        </div>
      </section>

      <p v-if="loaded && !servers.length" class="text-sm text-muted">No systems yet. Run an agent or <code class="text-faint">scripts/sim-agents.sh</code>.</p>
    </div>

    <div v-if="selected.size" class="fixed inset-x-0 bottom-4 z-30 mx-auto w-fit">
      <div class="flex items-center gap-4 rounded-xl border border-line bg-surface2 px-4 py-2.5 shadow-2xl">
        <span class="text-sm text-fg"><span class="font-semibold text-accent">{{ selected.size }}</span> selected</span>
        <div class="h-4 w-px bg-line"></div>
        <button @click="bulkDelete" class="flex items-center gap-1.5 rounded-lg bg-down/15 px-3 py-1.5 text-sm font-medium text-down hover:bg-down/25"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>Delete</button>
        <button @click="selected.clear()" class="text-sm text-muted hover:text-fg">Cancel</button>
      </div>
    </div>

    <AddSystemModal v-if="showAdd" @close="showAdd = false; load()" />
  </AppShell>
</template>
