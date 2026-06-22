<script setup>
import { ref, reactive, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import Gauge from '../components/Gauge.vue'
import AddSystemModal from '../components/AddSystemModal.vue'
import SystemSearch from '../components/SystemSearch.vue'
import FleetCharts from '../components/FleetCharts.vue'
import { encodeZoom, decodeZoom } from '../lib/zoom'
import { insertGaps } from '../lib/gaps'

const showAdd = ref(false)

const route = useRoute()
const router = useRouter()
const servers = ref([])
const loaded = ref(false) // true after the first successful load
const error = ref('')
const q = ref(route.query.q || '')
let qTimer
watch(q, (v) => { clearTimeout(qTimer); qTimer = setTimeout(() => router.replace({ query: { ...route.query, q: v || undefined } }), 300) })
// keep q in sync with the URL too, so navigating to a clean "/" (e.g. clicking
// "Systems") actually clears the chips instead of the stale ref re-adding them
watch(() => route.query.q, (v) => { if ((v || '') !== q.value) q.value = v || '' })
// namespace filter from URL (?ns=a,b ; empty = all) — shared/persisted, set in the sidebar
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const inNs = (s) => selectedNs.value.length === 0 || selectedNs.value.includes(s.namespace)
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
const pct = (u, t) => (u != null && t ? Math.round((u / t) * 100) : null)
const online = (s) => !!s.last_seen && Date.now() - new Date(s.last_seen).getTime() < 60000
const LATEST = computed(() => servers.value.map((s) => s.agent_version).filter(Boolean).sort(cmpVer).pop())
function cmpVer(a, b) { const p = (x) => x.split('.').map(Number); const A = p(a), B = p(b); for (let i = 0; i < 3; i++) if ((A[i]||0)!==(B[i]||0)) return (A[i]||0)-(B[i]||0); return 0 }
function agentCls(v) { if (!v) return 'bg-surface2 text-faint'; if (v === LATEST.value) return 'bg-accent/10 text-accent'; return cmpVer(v, '0.7.0') >= 0 ? 'bg-amber-500/10 text-amber-500' : 'bg-red-500/10 text-red-500' }

// Query mini-language: "cpu>50 disk<30 status:online kind:docker ns:prod web" (space = AND)
function parseQuery(qs) {
  return (qs || '').trim().split(/\s+/).filter(Boolean).map((tok) => {
    let m = tok.match(/^(cpu|mem|disk)(>=|<=|>|<|=)(\d+(?:\.\d+)?)$/i)
    if (m) return { t: 'num', f: m[1].toLowerCase(), op: m[2], v: +m[3] }
    m = tok.match(/^(status|kind|type|cluster|ns|agent|kernel|name|node|system):(.+)$/i)
    if (m) return { t: 'kv', k: m[1].toLowerCase(), v: m[2].toLowerCase() }
    return { t: 'text', v: tok.toLowerCase() }
  })
}
const metricVal = (s, f) => (f === 'cpu' ? s.cpu_percent : f === 'mem' ? pct(s.mem_used, s.mem_total) : pct(s.disk_used, s.disk_total))
const cmpOp = (a, op, b) => (op === '>' ? a > b : op === '<' ? a < b : op === '>=' ? a >= b : op === '<=' ? a <= b : a === b)
// wildcard match: substring by default; '*' acts as a glob (web*, *-prod, db*1)
const escapeRe = (s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')
function wild(hay, pat) {
  if (!pat) return true
  hay = (hay || '').toLowerCase()
  if (pat.includes('*')) return new RegExp('^' + pat.split('*').map(escapeRe).join('.*') + '$').test(hay)
  return hay.includes(pat)
}
function matchPred(s, p) {
  if (p.t === 'num') { const v = metricVal(s, p.f); return v != null && cmpOp(v, p.op, p.v) }
  if (p.t === 'kv') {
    if (['name', 'node', 'system'].includes(p.k)) return wild(s.name, p.v)
    if (p.k === 'status') return (online(s) ? 'online' : 'offline').startsWith(p.v)
    if (p.k === 'kind' || p.k === 'type') return s.kind === p.v
    if (p.k === 'cluster') return wild(s.cluster, p.v)
    if (p.k === 'ns') return wild(s.namespace, p.v)
    if (p.k === 'agent') return wild(s.agent_version, p.v)
    if (p.k === 'kernel') return wild(s.kernel, p.v)
  }
  // default (plain text) = node name (+ hostname), wildcard-aware
  return wild(s.name + ' ' + (s.hostname || ''), p.v)
}
// committed filters shown as chips (each token in q); search box appends via @add
const chips = computed(() => q.value.trim().split(/\s+/).filter(Boolean))
function addToken(tok) { const t = (tok || '').trim(); if (t) q.value = q.value.trim() ? `${q.value.trim()} ${t}` : t }
function removeChip(i) { const a = chips.value.slice(); a.splice(i, 1); q.value = a.join(' ') }
// reset clears both the text filters (?q) and the pinned-node selection (?fsel)
function resetFilters() { q.value = ''; selected.clear(); router.replace({ query: { ...route.query, q: undefined, fzoom: undefined } }) }
const shortName = (n) => (n && n.length > 12 ? n.slice(0, 12) + '…' : n)
const preds = computed(() => parseQuery(q.value))
const visible = computed(() => servers.value.filter((s) => inNs(s) && preds.value.every((p) => matchPred(s, p))))
function sortList(list, st) {
  const f = {
    name: (a, b) => a.name.localeCompare(b.name),
    type: (a, b) => (a.kind || '').localeCompare(b.kind || '') || a.name.localeCompare(b.name),
    cluster: (a, b) => (a.cluster || '').localeCompare(b.cluster || '') || a.name.localeCompare(b.name),
    ns: (a, b) => (a.namespace || '').localeCompare(b.namespace || ''),
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
  return { online: on, total: all.length, cpu: avg(all, (x) => x.cpu_percent), mem: avg(all, (x) => pct(x.mem_used, x.mem_total)), nodes: all.filter((s) => s.kind === 'node').length }
})

function sortBy(col) { if (sortState.col === col) sortState.dir = sortState.dir === 'asc' ? 'desc' : 'asc'; else { sortState.col = col; sortState.dir = 'asc' } }
const arrow = (col) => (sortState.col === col ? (sortState.dir === 'desc' ? ' ↓' : ' ↑') : '')
// click a row attribute (type/cluster/ns) → set that filter dimension (replacing any existing)
function setFilter(key, val) { const toks = chips.value.filter((t) => !t.toLowerCase().startsWith(key + ':')); toks.push(`${key}:${val}`); q.value = toks.join(' ') }
function toggleRow(id) { selected.has(id) ? selected.delete(id) : selected.add(id) }
function toggleAll(rows) { const all = rows.length && rows.every((s) => selected.has(s.id)); rows.forEach((s) => (all ? selected.delete(s.id) : selected.add(s.id))) }
function toggleExpand(k) { expanded.has(k) ? expanded.delete(k) : expanded.add(k) }
async function bulkDelete() { for (const id of [...selected]) { try { await api.del(`/api/systems/${id}`) } catch {} } selected.clear(); await load() }

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
// overlay only the hosts that pass the current filter + namespace
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

async function load() {
  try {
    servers.value = await api.get('/api/systems')
    error.value = ''
    loaded.value = true
  } catch {
    // Keep showing existing data on a transient poll failure; only surface an
    // error before the first successful load.
    if (!loaded.value) error.value = 'Failed to load systems'
  }
}
onMounted(() => { load(); loadFleet(); timer = setInterval(() => { load(); loadFleet() }, 5000) })
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
  <AppShell title="Systems">
    <div class="space-y-5">
      <!-- hero -->
      <section class="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="text-xs uppercase tracking-wider text-muted">Systems online</div>
          <div class="mt-1.5 text-2xl font-semibold text-fg">{{ hero.online }}<span class="text-sm text-faint"> / {{ hero.total }}</span></div>
          <div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.total ? (hero.online / hero.total) * 100 : 0) + '%' }"></div></div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-muted">Nodes</div><div class="mt-1.5 text-2xl font-semibold text-fg">{{ hero.nodes }}</div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-muted">Avg CPU</div><div class="mt-1.5 text-2xl font-semibold text-fg">{{ hero.cpu ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.cpu || 0) + '%' }"></div></div></div>
        <div class="rounded-xl border border-line bg-surface p-4"><div class="text-xs uppercase tracking-wider text-muted">Avg memory</div><div class="mt-1.5 text-2xl font-semibold text-fg">{{ hero.mem ?? '—' }}%</div><div class="mt-2 h-1 overflow-hidden rounded bg-line"><div class="h-full bg-accent" :style="{ width: (hero.mem || 0) + '%' }"></div></div></div>
      </section>

      <!-- toolbar -->
      <div class="flex flex-wrap items-center justify-between gap-3">
        <SystemSearch :items="servers" @add="addToken" />
        <button @click="showAdd = true" class="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> Add system</button>
      </div>

      <p v-if="!loaded && !error" class="text-sm text-muted">Loading…</p>
      <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

      <!-- Fleet overlay: every visible host on one chart per metric (filter applies) -->
      <section v-if="servers.length">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <h2 class="text-sm font-semibold text-fg">Fleet metrics</h2>
          <span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ visible.length }} hosts</span>
          <!-- active filter chips (each token in the query) + reset -->
          <span v-for="(c, i) in chips" :key="c + i" class="flex items-center gap-1 rounded-full border border-line bg-surface2 py-0.5 pl-2 pr-1 text-xs text-fg">
            <span class="tabular-nums">{{ c }}</span>
            <button @click="removeChip(i)" title="Remove filter" class="grid h-4 w-4 place-items-center rounded-full text-faint hover:bg-red-500/15 hover:text-red-500"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </span>
          <!-- selected nodes (row checkbox) — shown on charts, listed as chips -->
          <span v-for="s in pinnedSystems" :key="'pin-' + s.id" :title="s.name" class="flex items-center gap-1 rounded-full border border-accent/40 bg-accent/10 py-0.5 pl-2 pr-1 text-xs text-accent">
            <span class="h-2 w-2 rounded-full" :style="{ background: colorOf[s.name] }"></span>
            <span class="tabular-nums">{{ shortName(s.name) }}</span>
            <button @click="toggleRow(s.id)" title="Deselect" class="grid h-4 w-4 place-items-center rounded-full hover:bg-accent/25"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </span>
          <button v-if="chips.length || pinnedSystems.length || fviewRange" @click="resetFilters" class="text-xs text-muted hover:text-accent">Reset</button>
          <!-- range selector: a drag-zoom shows the custom window here as a chip -->
          <div class="ml-auto flex items-center gap-2">
            <span v-if="fviewRange" class="flex items-center gap-1 rounded-lg border border-accent/40 bg-accent/10 py-1 pl-2 pr-1 text-xs text-accent">
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>
              <span class="tabular-nums">{{ fmtTs(fviewRange[0]) }} – {{ fmtTs(fviewRange[1]) }}</span>
              <button @click="setFzoom(null)" title="Clear zoom" class="grid h-4 w-4 place-items-center rounded-full hover:bg-accent/25"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
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

      <!-- Hosts: one flat table; Type / Cluster / Namespace are clickable filters -->
      <section v-if="rows.length">
        <div class="mb-2 flex items-center gap-2"><h2 class="text-sm font-semibold text-fg">Hosts</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ rows.length }}</span></div>
        <div class="overflow-x-auto rounded-xl border border-line">
          <table class="w-full min-w-[1040px] text-sm">
            <thead class="border-b border-line bg-surface text-left text-xs uppercase tracking-wider text-muted"><tr>
              <th class="w-8 px-3 py-2.5"><input type="checkbox" :checked="rows.length && rows.every((s)=>selected.has(s.id))" @change="toggleAll(rows)" class="h-4 w-4 accent-accent" /></th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('name')">Host{{ arrow('name') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('ns')">Namespace{{ arrow('ns') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('type')">Type{{ arrow('type') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('cluster')">Cluster{{ arrow('cluster') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('status')">Status{{ arrow('status') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('cpu')">CPU{{ arrow('cpu') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('mem')">Memory{{ arrow('mem') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('disk')">Disk{{ arrow('disk') }}</th>
              <th class="cursor-pointer select-none px-4 py-2.5 font-medium hover:text-fg" @click="sortBy('agent')">Agent{{ arrow('agent') }}</th>
            </tr></thead>
            <tbody>
              <template v-for="s in rows" :key="s.id">
                <tr class="lm-row border-b border-line" :class="selected.has(s.id) ? 'sel' : ''" @mouseenter="onLegendHover(s.name)" @mouseleave="onLegendHover(null)">
                  <td class="px-3 py-3"><input type="checkbox" :checked="selected.has(s.id)" @change="toggleRow(s.id)" class="h-4 w-4 accent-accent" /></td>
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1.5">
                      <button v-if="s.kind === 'docker'" @click="toggleDocker(s)" class="text-muted hover:text-accent"><svg class="h-4 w-4 transition-transform" :class="expanded.has(s.id) ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></button>
                      <span v-else class="w-4 shrink-0"></span>
                      <span class="h-2 w-2 shrink-0 rounded-full" :title="online(s) ? 'online' : 'offline'" :style="{ background: colorOf[s.name] }"></span>
                      <RouterLink :to="detailLink(s)" class="text-fg hover:text-accent">{{ s.name }}</RouterLink>
                    </div>
                  </td>
                  <td class="px-4 py-3"><button @click="setFilter('ns', s.namespace)" :title="`Filter ns:${s.namespace}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.namespace || '—' }}</button></td>
                  <td class="px-4 py-3"><button @click="setFilter('kind', s.kind)" :title="`Filter kind:${s.kind}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ KIND_LABEL[s.kind] || s.kind }}</button></td>
                  <td class="px-4 py-3"><button v-if="s.cluster" @click="setFilter('cluster', s.cluster)" :title="`Filter cluster:${s.cluster}`" class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-muted hover:text-accent">{{ s.cluster }}</button><span v-else class="text-faint">—</span></td>
                  <td class="px-4 py-3"><button @click="setFilter('status', online(s)?'online':'offline')" :title="`Filter status:${online(s)?'online':'offline'}`" class="text-sm hover:underline" :class="online(s)?'text-accent':'text-red-500'">{{ online(s)?'online':'offline' }}</button></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?r(s.cpu_percent):null" /></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?pct(s.mem_used,s.mem_total):null" /></td>
                  <td class="px-4 py-3"><Gauge :v="online(s)?pct(s.disk_used,s.disk_total):null" /></td>
                  <td class="px-4 py-3"><span class="rounded px-1.5 py-0.5 text-xs" :class="agentCls(s.agent_version)">{{ s.agent_version ? 'v'+s.agent_version : '—' }}</span></td>
                </tr>
                <tr v-for="c in (containers[s.id] || [])" v-show="s.kind === 'docker' && expanded.has(s.id)" :key="s.id + ':' + c.name" class="lm-row border-b border-line bg-bg/40">
                  <td></td>
                  <td class="px-4 py-2"><RouterLink :to="`/system/${s.id}?type=container&name=${encodeURIComponent(c.name)}&parent=${encodeURIComponent(s.name)}&ptype=docker`" class="flex items-center gap-2 pl-10 text-sm text-fg hover:text-accent"><span class="text-faint">└</span>{{ c.name }}</RouterLink></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2"><span class="rounded bg-surface2 px-1.5 py-0.5 text-xs text-faint">container</span></td>
                  <td class="px-4 py-2 text-faint">—</td>
                  <td class="px-4 py-2 text-sm text-accent">running</td>
                  <td class="px-4 py-2"><Gauge :v="c.cpu" /></td>
                  <td class="px-4 py-2 tabular-nums text-muted">{{ c.mem != null ? (c.mem / 1048576).toFixed(0) + ' MB' : '—' }}</td>
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
        <button @click="bulkDelete" class="flex items-center gap-1.5 rounded-lg bg-red-500/15 px-3 py-1.5 text-sm font-medium text-red-500 hover:bg-red-500/25"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>Delete</button>
        <button @click="selected.clear()" class="text-sm text-muted hover:text-fg">Cancel</button>
      </div>
    </div>

    <AddSystemModal v-if="showAdd" @close="showAdd = false; load()" />
  </AppShell>
</template>
