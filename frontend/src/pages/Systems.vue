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
import FleetCharts from '../components/FleetCharts.vue'
import { encodeZoom, decodeZoom } from '../lib/zoom'
import { insertGaps } from '../lib/gaps'
import { pct, online, parseQuery, matchPred } from '../lib/hostFilter'

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
const KIND_LABEL = { node: 'Node', docker: 'Docker', k8s: 'K8s' }
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
function resetFilters() { q.value = ''; selected.clear(); router.replace({ query: { ...route.query, q: undefined, fzoom: undefined } }) }
const shortName = (n) => (n && n.length > 12 ? n.slice(0, 12) + '…' : n)
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
const hero = computed(() => {
  const all = visible.value, on = all.filter(online).length
  return { online: on, total: all.length, cpu: avg(all, (x) => x.cpu_percent), mem: avg(all, (x) => pct(x.mem_used, x.mem_total)), disk: avg(all, (x) => pct(x.disk_used, x.disk_total)) }
})

// ---- thresholds + "needs attention" triage --------------------------------
const DEFAULT_THR = { cpu_warn: 80, cpu_crit: 90, mem_warn: 80, mem_crit: 90, disk_warn: 80, disk_crit: 90, dutil_warn: 80, dutil_crit: 95 }
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
const issueText = (i) => (i.key === 'down' ? 'Offline — not reporting in' : `High ${ISSUE[i.key].label.toLowerCase()}: ${i.val}% (${i.crit ? 'critical' : 'warning'})`)
const chipTitle = (h) => `${h.s.name} · ${h.s.workspace}\n` + h.issues.map(issueText).join('\n')
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

// ---- Fleet overlay (NewRelic-style: every visible host on one chart per metric) ----
const FRANGES = ['30m', '1h', '3h', '6h', '12h', '24h']
const FSPAN = { '30m': 1800, '1h': 3600, '3h': 10800, '6h': 21600, '12h': 43200, '24h': 86400 }
const frange = computed(() => route.query.frange || '30m')
function setFrange(r) { router.replace({ query: { ...route.query, frange: r, fzoom: undefined } }) }
// drag-zoom window persisted in the URL as a human-readable range, shared by all fleet charts
const fviewRange = computed(() => decodeZoom(route.query.fzoom))
function setFzoom(r) { router.replace({ query: { ...route.query, fzoom: encodeZoom(r) } }) }
// header: hovered point → its time; zoomed → the selected range; else → "now"
const headerTime = computed(() => fleetTime.value || (fviewRange.value ? `${fmtTs(fviewRange.value[0])} – ${fmtTs(fviewRange.value[1])}` : 'now'))
const fleet = ref(null)
async function loadFleet() { try { fleet.value = await api.get(`/api/fleet?range=${frange.value}`) } catch {} }
// stable host → color map (by sorted name) so chart lines and table dots match
const colorOf = computed(() => {
  const names = [...new Set(servers.value.map((s) => s.name))].sort()
  const m = {}
  names.forEach((n, i) => { m[n] = `hsl(${(i * 47) % 360} 70% 58%)` })
  return m
})
// overlay only the hosts that pass the current filter + workspace
const visibleNames = computed(() => new Set(visible.value.map((s) => s.name)))
const fleetSeries = (arr) => (arr || []).filter((s) => visibleNames.value.has(s.name)).map((s) => ({ name: s.name, color: colorOf.value[s.name] || '#888', data: s.data }))
// Selection is unified: the row checkbox (`selected`, by id) both marks for
// bulk-delete AND isolates the node on the charts. Hover a row → transient highlight.
const hoverNode = ref(null)
// Debounce hover→isolate: only refocus the charts once the cursor settles for a
// moment, so flicking across nodes doesn't strobe the graphs. Leaving clears now.
let hoverTimer = null
function onLegendHover(name) {
  clearTimeout(hoverTimer)
  if (!name) { hoverNode.value = null; return }
  hoverTimer = setTimeout(() => { hoverNode.value = name }, 500)
}
const fleetTime = ref('') // hovered timestamp (empty when not hovering)
const fmtTs = (ts) => new Date(ts * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })
const pinnedSystems = computed(() => servers.value.filter((s) => selected.has(s.id)))
const selectedNames = computed(() => pinnedSystems.value.map((s) => s.name))
const fleetFocus = computed(() => (hoverNode.value ? [hoverNode.value] : selectedNames.value.length ? selectedNames.value : null))
function toggleByName(name) { const s = servers.value.find((x) => x.name === name); if (s) toggleRow(s.id) }
// rebuild fleet data with null breaks inserted at timeline gaps (agents stopped)
const gappedFleet = computed(() => {
  const f = fleet.value
  if (!f || !f.t || f.t.length < 3) return f
  const groups = ['cpu', 'mem', 'disk', 'net']
  const arrays = [], map = []
  groups.forEach((g) => (f[g] || []).forEach((s) => { arrays.push(s.data); map.push([g, s.name]) }))
  const { t, arrays: na } = insertGaps(f.t, arrays)
  const out = { t, cpu: [], mem: [], disk: [], net: [] }
  map.forEach(([g, name], k) => out[g].push({ name, data: na[k] }))
  return out
})
const fleetCharts = computed(() => {
  const f = gappedFleet.value
  if (!f) return []
  return [
    { title: 'CPU', unit: '%', series: fleetSeries(f.cpu) },
    { title: 'Memory', unit: '%', series: fleetSeries(f.mem) },
    { title: 'Disk', unit: '%', series: fleetSeries(f.disk) },
    { title: 'Network', unit: 'B/s', series: fleetSeries(f.net) },
  ]
})

// `/api/systems` is global (not workspace-scoped), so one cache key — navigating
// back to Systems paints the last fleet instantly, then revalidates silently.
const { loaded, reload: load } = useCached({
  key: () => 'systems',
  load: () => api.get('/api/systems'),
  apply: (list) => { servers.value = list; error.value = '' },
  // Keep showing existing data on a transient poll failure; only surface an
  // error before the first successful load.
  onError: () => { if (!servers.value.length) error.value = 'Failed to load systems' },
})
onMounted(() => { load(); loadFleet(); loadThresholds(); timer = setInterval(() => { load(); loadFleet() }, 5000) })
onUnmounted(() => clearInterval(timer))
watch(frange, loadFleet)

// a k8s row IS a node → open its node detail (with cluster breadcrumb), not the cluster aggregate
const detailLink = (s) => {
  const n = encodeURIComponent(s.name)
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
          <div class="text-xs uppercase tracking-wider text-faint">Systems online</div>
          <div class="mt-1.5 font-mono text-metric text-fg">{{ hero.online }}<span class="text-sm text-faint"> / {{ hero.total }}</span></div>
          <div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.total ? (hero.online / hero.total) * 100 : 0) + '%' }"></div></div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg disk</div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.disk ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.disk || 0) + '%' }"></div></div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg CPU</div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.cpu ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.cpu || 0) + '%' }"></div></div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-faint">Avg memory</div><div class="mt-1.5 font-mono text-metric text-fg">{{ hero.mem ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.mem || 0) + '%' }"></div></div></div>
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
            <span v-for="i in h.issues" :key="i.key" v-tip="issueText(i)"
              class="inline-flex items-center gap-0.5 font-mono tabular-nums" :class="i.crit ? 'text-down' : 'text-warn'">
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path :d="ISSUE[i.key].icon"/></svg>
              <span v-if="i.val != null">{{ i.val }}%</span>
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

      <!-- Fleet overlay: every visible host on one chart per metric (filter applies) -->
      <section v-if="servers.length">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <h2 class="text-sm font-semibold text-fg">Fleet metrics</h2>
          <span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ visible.length }} hosts</span>
          <!-- active filter chips (each token in the query) + reset -->
          <span v-for="(c, i) in chips" :key="c + i" class="flex items-center gap-1 rounded-full border border-line bg-surface2 py-0.5 pl-2 pr-1 text-xs text-fg">
            <span class="font-mono tabular-nums">{{ c }}</span>
            <button @click="removeChip(i)" v-tip="`Remove filter`" class="grid h-4 w-4 place-items-center rounded-full text-faint hover:bg-down/15 hover:text-down"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </span>
          <!-- selected nodes (row checkbox) — shown on charts, listed as chips -->
          <span v-for="s in pinnedSystems" :key="'pin-' + s.id" v-tip="s.name" class="flex items-center gap-1 rounded-full border border-accent/40 bg-accent/10 py-0.5 pl-2 pr-1 text-xs text-accent">
            <span class="h-2 w-2 rounded-full" :style="{ background: colorOf[s.name] }"></span>
            <span class="font-mono tabular-nums">{{ shortName(s.name) }}</span>
            <button @click="toggleRow(s.id)" v-tip="`Deselect`" class="grid h-4 w-4 place-items-center rounded-full hover:bg-accent/25"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </span>
          <button v-if="chips.length || pinnedSystems.length || fviewRange" @click="resetFilters" class="text-xs text-muted hover:text-accent">Reset</button>
          <!-- range selector: a drag-zoom shows the custom window here as a chip -->
          <div class="ml-auto flex items-center gap-2">
            <span v-if="fviewRange" class="flex items-center gap-1 rounded-lg border border-accent/40 bg-accent/10 py-1 pl-2 pr-1 text-xs text-accent">
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>
              <span class="font-mono tabular-nums">{{ fmtTs(fviewRange[0]) }} – {{ fmtTs(fviewRange[1]) }}</span>
              <button @click="setFzoom(null)" v-tip="`Clear zoom`" class="grid h-4 w-4 place-items-center rounded-full hover:bg-accent/25"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
            </span>
            <div v-else class="flex rounded-lg border border-line bg-surface2 p-0.5 text-xs">
              <button v-for="rr in FRANGES" :key="rr" @click="setFrange(rr)" class="rounded-md px-2.5 py-1" :class="frange===rr?'bg-accent/15 font-medium text-accent':'text-muted hover:text-fg'">{{ rr }}</button>
            </div>
          </div>
        </div>
        <p v-if="!visible.length" class="rounded-xl border border-line bg-surface p-4 text-sm text-muted">No hosts match the filter. <button @click="resetFilters" class="text-accent hover:underline">Reset</button></p>
        <FleetCharts v-else :charts="fleetCharts" :time="gappedFleet?.t || []" :span-seconds="FSPAN[frange]" :view-range="fviewRange"
          :focus-names="fleetFocus" :selected-names="selectedNames" sync-key="fleet"
          @legend-hover="onLegendHover" @legend-toggle="toggleByName" @zoom="setFzoom" />
      </section>

      <!-- Hosts: one flat table; Type / Cluster / Workspace are clickable filters -->
      <section v-if="rows.length">
        <div class="mb-2 flex items-center gap-2"><h2 class="text-sm font-semibold text-fg">Hosts</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ rows.length }}</span></div>
        <div class="overflow-x-auto rounded-xl border border-line">
          <table class="w-full min-w-[1040px] text-sm">
            <thead class="border-b border-line2 bg-head text-left text-xs uppercase tracking-wide text-fg"><tr>
              <th class="w-8 px-3 py-2.5"><input type="checkbox" :checked="rows.length && rows.every((s)=>selected.has(s.id))" @change="toggleAll(rows)" class="h-4 w-4 accent-accent" /></th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('name')">Host{{ arrow('name') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('ws')">Workspace{{ arrow('ws') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('type')">Type{{ arrow('type') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('cluster')">Cluster{{ arrow('cluster') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('status')">Status{{ arrow('status') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('cpu')">CPU{{ arrow('cpu') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('mem')">Memory{{ arrow('mem') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('disk')">Disk{{ arrow('disk') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-extrabold hover:text-fg" @click="sortBy('agent')">Agent{{ arrow('agent') }}</th>
            </tr></thead>
            <tbody>
              <template v-for="s in rows" :key="s.id">
                <tr class="vantage-row border-b border-line border-l-2" :class="[selected.has(s.id) ? 'sel' : '', sevOf(s) === 3 || sevOf(s) === 2 ? 'border-l-down' : sevOf(s) === 1 ? 'border-l-warn' : 'border-l-transparent']" @mouseenter="onLegendHover(s.name)" @mouseleave="onLegendHover(null)">
                  <td class="px-3 py-3"><input type="checkbox" :checked="selected.has(s.id)" @change="toggleRow(s.id)" class="h-4 w-4 accent-accent" /></td>
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1.5">
                      <button v-if="s.kind === 'docker'" @click="toggleDocker(s)" class="text-muted hover:text-accent"><svg class="h-4 w-4 transition-transform" :class="expanded.has(s.id) ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></button>
                      <span v-else class="w-4 shrink-0"></span>
                      <span class="h-2 w-2 shrink-0 rounded-full" v-tip="online(s) ? 'online' : 'offline'" :style="{ background: colorOf[s.name] }"></span>
                      <RouterLink :to="detailLink(s)" class="font-mono text-fg hover:text-accent">{{ s.name }}</RouterLink>
                    </div>
                  </td>
                  <td class="px-4 py-3"><button @click="setFilter('ws', s.workspace)" v-tip="`Filter ws:${s.workspace}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.workspace || '—' }}</button></td>
                  <td class="px-4 py-3"><button @click="setFilter('kind', s.kind)" v-tip="`Filter kind:${s.kind}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ KIND_LABEL[s.kind] || s.kind }}</button></td>
                  <td class="px-4 py-3"><button v-if="s.cluster" @click="setFilter('cluster', s.cluster)" v-tip="`Filter cluster:${s.cluster}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.cluster }}</button><span v-else class="text-faint">—</span></td>
                  <td class="px-4 py-3"><button @click="setFilter('status', online(s)?'online':'offline')" v-tip="`Filter status:${online(s)?'online':'offline'}`" class="text-sm hover:underline" :class="online(s)?'text-accent':'text-down'">{{ online(s)?'online':'offline' }}</button></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?r(s.cpu_percent):null" /></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?pct(s.mem_used,s.mem_total):null" /></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?pct(s.disk_used,s.disk_total):null" /></td>
                  <td class="px-4 py-3"><span class="rounded px-1.5 py-0.5 text-xs" :class="agentCls(s.agent_version)">{{ s.agent_version ? 'v'+s.agent_version : '—' }}</span></td>
                </tr>
                <tr v-for="c in (containers[s.id] || [])" v-show="s.kind === 'docker' && expanded.has(s.id)" :key="s.id + ':' + c.name" class="vantage-row border-b border-line bg-bg/40">
                  <td></td>
                  <td class="px-4 py-2"><RouterLink :to="`/system/${s.id}?type=container&name=${encodeURIComponent(c.name)}&parent=${encodeURIComponent(s.name)}&ptype=docker`" class="flex items-center gap-2 pl-10 font-mono text-sm text-fg hover:text-accent"><span class="text-faint">└</span>{{ c.name }}</RouterLink></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2"><span class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-faint">container</span></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2 text-sm text-accent">running</td>
                  <td class="px-4 py-2"><Gauge :v="c.cpu" /></td>
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
