<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import UplotChart from '../components/UplotChart.vue'
import SystemSearch from '../components/SystemSearch.vue'
import Gauge from '../components/Gauge.vue'
import { encodeZoom, decodeZoom } from '../lib/zoom'
import { insertGaps } from '../lib/gaps'

const route = useRoute()
const router = useRouter()
const id = computed(() => route.params.id)
const type = computed(() => route.query.type || 'node')
const name = computed(() => route.query.name || id.value)
const parent = computed(() => route.query.parent || '')
const ptype = computed(() => route.query.ptype || '')
const TYPE_LABEL = { node: 'Node', host: 'Host', docker: 'Docker', k8s: 'Kubernetes', container: 'Container' }
// breadcrumb category → Systems list filtered to that kind (so every crumb is a real link)
const KIND_OF = { node: 'node', host: 'docker', docker: 'docker', k8s: 'k8s', container: 'docker' }
const kindHref = (t) => ({ path: '/', query: { q: `kind:${KIND_OF[t] || 'node'}` } })
const RANGES = [['30m', '1m'], ['1h', '1m'], ['3h', '1m'], ['6h', '5m'], ['12h', '5m'], ['24h', '15m']]
// range persisted in the URL so F5 keeps it
const range = computed(() => route.query.range || '30m')
const resOf = computed(() => RANGES.find(([r]) => r === range.value)?.[1] || '1m')
function setRange(r) { router.replace({ query: { ...route.query, range: r, zoom: undefined } }) }
// drag-zoom window persisted in the URL as a human-readable range, shared by all charts
const viewRange = computed(() => decodeZoom(route.query.zoom))
function setZoom(r) { router.replace({ query: { ...route.query, zoom: encodeZoom(r) } }) }
const SPAN = { '30m': 1800, '1h': 3600, '3h': 10800, '6h': 21600, '12h': 43200, '24h': 86400 }
// charts always span the full selected window (blank where data is missing)
const spanSeconds = computed(() => SPAN[range.value] || 0)

// legend interaction: hover a metric → show only it in its chart; click → pin
// (multi), persisted in the URL (?sel). Selection is per-metric, applied to the
// chart that owns the metric.
const selectedMetrics = computed(() => (route.query.sel || '').split(',').filter(Boolean))
const hoverMetric = ref(null)
const chartTime = ref('') // hovered timestamp (empty when not hovering)
const fmtTs = (ts) => new Date(ts * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })
// header shows: hovered point → its time; zoomed → the selected range; else → "now"
const headerTime = computed(() => chartTime.value || (viewRange.value ? `${fmtTs(viewRange.value[0])} – ${fmtTs(viewRange.value[1])}` : 'now'))
function toggleMetric(name) {
  const set = new Set(selectedMetrics.value)
  set.has(name) ? set.delete(name) : set.add(name)
  router.replace({ query: { ...route.query, sel: [...set].join(',') || undefined } })
}
function chartFocus(series) {
  if (hoverMetric.value && series.some((s) => s.name === hoverMetric.value)) return [hoverMetric.value]
  const sel = series.filter((s) => selectedMetrics.value.includes(s.name)).map((s) => s.name)
  return sel.length ? sel : null
}

const metrics = ref(null)
const containersList = ref([])
const containersTime = ref([])
const clusterNodes = ref([])
const error = ref('')

const C = { teal: '#34E1C4', amber: '#F4A261', blue: '#5BA8FF', purple: '#8B7FD6' }
const lastVal = (d) => { if (!d) return null; for (let i = d.length - 1; i >= 0; i--) if (d[i] != null) return d[i]; return null }
const pct = (u, t) => (u != null && t ? Math.round((u / t) * 100) : null)
const online = (s) => !!s.last_seen && Date.now() - new Date(s.last_seen).getTime() < 60000

// metrics with null breaks inserted at timeline gaps (so a stopped agent leaves a
// blank instead of a line bridged across the gap)
const gappedMetrics = computed(() => {
  const m = metrics.value
  if (!m || !m.t || m.t.length < 3) return m
  const keys = ['cpu', 'mem_pct', 'disk_pct', 'net_rx', 'net_tx', 'dr', 'dw', 'load1', 'load5', 'load15', 'cpu_user', 'cpu_system', 'cpu_iowait', 'cpu_steal'].filter((k) => Array.isArray(m[k]))
  const { t, arrays } = insertGaps(m.t, keys.map((k) => m[k]))
  const out = { ...m, t }
  keys.forEach((k, i) => { out[k] = arrays[i] })
  return out
})
const hostCharts = computed(() => {
  const m = gappedMetrics.value
  if (!m) return []
  // One CPU chart that folds the breakdown in: overall line + user/system/iowait/
  // steal (Linux /proc/stat). macOS has no breakdown → just the overall line.
  const hasBreakdown = m.cpu_user && m.cpu_user.some((v) => v > 0)
  const cpuSeries = [{ name: 'CPU', color: C.teal, data: m.cpu }]
  if (hasBreakdown) {
    cpuSeries.push(
      { name: 'user', color: C.blue, data: m.cpu_user },
      { name: 'system', color: C.amber, data: m.cpu_system },
      { name: 'iowait', color: C.purple, data: m.cpu_iowait },
      { name: 'steal', color: '#e06c9f', data: m.cpu_steal },
    )
  }
  const charts = [
    { title: 'CPU', sub: hasBreakdown ? 'overall + user / system / iowait / steal' : 'overall %', unit: '%', area: !hasBreakdown, series: cpuSeries },
  ]
  if (m.load1 && m.load1.length) {
    charts.push({
      title: 'Load average', sub: '1m / 5m / 15m', unit: '',
      series: [
        { name: '1m', color: C.teal, data: m.load1 },
        { name: '5m', color: C.amber, data: m.load5 },
        { name: '15m', color: C.blue, data: m.load15 },
      ],
    })
  }
  charts.push(
    { title: 'Memory', sub: 'used %', unit: '%', series: [{ name: 'Memory', color: C.blue, data: m.mem_pct }] },
    { title: 'Disk Usage', sub: 'used %', unit: '%', series: [{ name: 'Disk', color: C.purple, data: m.disk_pct }] },
    { title: 'Disk I/O', sub: 'read / write', unit: 'B/s', series: [{ name: 'read', color: C.teal, data: m.dr }, { name: 'write', color: C.amber, data: m.dw }] },
    { title: 'Network', sub: 'rx / tx', unit: 'B/s', series: [{ name: 'rx', color: C.teal, data: m.net_rx }, { name: 'tx', color: C.blue, data: m.net_tx }] },
  )
  return charts
})
const snapshot = computed(() => {
  const m = metrics.value
  if (!m) return null
  return { cpu: Math.round(lastVal(m.cpu) ?? 0), mem: Math.round(lastVal(m.mem_pct) ?? 0), disk: Math.round(lastVal(m.disk_pct) ?? 0), dr: lastVal(m.dr), dw: lastVal(m.dw), rx: lastVal(m.net_rx), tx: lastVal(m.net_tx) }
})
const containerCharts = (cname) => {
  // built lazily for the container leaf
  return []
}

async function loadMetrics() {
  try { metrics.value = await api.get(`/api/systems/${id.value}/metrics?range=${range.value}`); error.value = '' }
  catch { if (!metrics.value) error.value = 'Failed to load metrics' }
}
async function loadContainers() {
  try {
    const h = await api.get(`/api/systems/${id.value}/containers?range=${range.value}`)
    containersTime.value = h.t || []
    const memByName = Object.fromEntries((h.mem || []).map((s) => [s.name, s]))
    const netByName = Object.fromEntries((h.net || []).map((s) => [s.name, s]))
    containersList.value = (h.cpu || []).map((s) => ({
      name: s.name, cpu: Math.round(lastVal(s.data) ?? 0),
      mem: lastVal(memByName[s.name]?.data), net: lastVal(netByName[s.name]?.data),
      series: { cpu: s.data, mem: memByName[s.name]?.data, net: netByName[s.name]?.data },
    }))
  } catch { containersList.value = [] }
}
async function loadCluster() {
  try {
    const all = await api.get('/api/systems')
    clusterNodes.value = all.filter((s) => s.kind === 'k8s' && (s.cluster || 'unnamed') === name.value)
  } catch { clusterNodes.value = [] }
}

// docker detail = a fleet scoped to one host: the host itself is just another
// line alongside its containers. Host + containers share the agent's push
// timestamps, so we align both on the union timeline.
const escapeRe = (s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')
const wild = (hay, pat) => { if (!pat) return true; hay = (hay || '').toLowerCase(); return pat.includes('*') ? new RegExp('^' + pat.split('*').map(escapeRe).join('.*') + '$').test(hay) : hay.includes(pat) }
const cmpOp = (a, op, b) => (op === '>' ? a > b : op === '<' ? a < b : op === '>=' ? a >= b : op === '<=' ? a <= b : a === b)
// filter chips (search) for the docker overlay, persisted in the URL (?q)
const chips = computed(() => (route.query.q || '').trim().split(/\s+/).filter(Boolean))
function addToken(tok) { const t = (tok || '').trim(); if (!t) return; const cur = (route.query.q || '').trim(); router.replace({ query: { ...route.query, q: cur ? `${cur} ${t}` : t } }) }
function removeChip(i) { const a = chips.value.slice(); a.splice(i, 1); router.replace({ query: { ...route.query, q: a.join(' ') || undefined } }) }
function resetFilters() { router.replace({ query: { ...route.query, q: undefined } }) }

const dockerOverlay = computed(() => {
  const m = metrics.value, ct = containersTime.value, list = containersList.value
  if (!m || !m.t || !m.t.length) return null
  const tset = new Set(m.t); ct.forEach((x) => tset.add(x))
  const t = [...tset].sort((a, b) => a - b)
  const hi = new Map(m.t.map((ts, i) => [ts, i])), ci = new Map(ct.map((ts, i) => [ts, i]))
  const pick = (arr, idx, ts) => { const i = idx.get(ts); return i == null || !arr ? null : (arr[i] ?? null) }
  const hostNet = (ts) => { const i = hi.get(ts); return i == null ? null : (m.net_rx?.[i] ?? 0) + (m.net_tx?.[i] ?? 0) }
  const h = name.value
  const cpu = [{ name: h, data: t.map((ts) => pick(m.cpu, hi, ts)) }]
  const mem = [{ name: h, data: t.map((ts) => pick(m.mem_used, hi, ts)) }]
  const disk = [{ name: h, data: t.map((ts) => pick(m.disk_pct, hi, ts)) }]
  const net = [{ name: h, data: t.map((ts) => hostNet(ts)) }]
  for (const c of list) {
    cpu.push({ name: c.name, data: t.map((ts) => pick(c.series.cpu, ci, ts)) })
    if (c.series.mem) mem.push({ name: c.name, data: t.map((ts) => pick(c.series.mem, ci, ts)) })
    if (c.series.net) net.push({ name: c.name, data: t.map((ts) => pick(c.series.net, ci, ts)) })
  }
  return { t, host: h, cpu, mem, disk, net }
})
// entity names (host + containers) passing the filter chips
const dockerPass = computed(() => {
  const o = dockerOverlay.value
  if (!o) return new Set()
  const last = (a) => { for (let i = a.length - 1; i >= 0; i--) if (a[i] != null) return a[i]; return null }
  const v = {}
  ;['cpu', 'mem', 'disk', 'net'].forEach((g) => o[g].forEach((s) => { (v[s.name] ||= {})[g] = last(s.data) }))
  const ok = (name, tok) => {
    const m = tok.match(/^(cpu|mem|disk|net)(>=|<=|>|<|=)(\d+(?:\.\d+)?)$/i)
    if (m) { const x = v[name]?.[m[1].toLowerCase()]; return x != null && cmpOp(x, m[2], +m[3]) }
    const kv = tok.match(/^(name|node):(.+)$/i)
    return wild(name, (kv ? kv[2] : tok).toLowerCase())
  }
  return new Set(Object.keys(v).filter((n) => chips.value.every((tok) => ok(n, tok))))
})
const dockerCharts = computed(() => {
  const o = dockerOverlay.value
  if (!o) return null
  const keep = dockerPass.value
  const arrays = [], map = []
  ;['cpu', 'mem', 'disk', 'net'].forEach((g) => o[g].filter((s) => keep.has(s.name)).forEach((s) => { arrays.push(s.data); map.push([g, s.name]) }))
  const { t, arrays: na } = insertGaps(o.t, arrays)
  const out = { cpu: [], mem: [], disk: [], net: [] }
  map.forEach(([g, name], i) => out[g].push({ name, data: na[i] }))
  const defs = [
    { title: 'CPU', sub: 'host + containers', unit: '%', series: out.cpu },
    { title: 'Memory', sub: 'host + containers', unit: 'B', series: out.mem },
    { title: 'Disk', sub: 'host', unit: '%', series: out.disk },
    { title: 'Network', sub: 'host + containers', unit: 'B/s', series: out.net },
  ].filter((c) => c.series.length)
  return { t, count: keep.size, charts: defs }
})
const containerLeaf = computed(() => containersList.value.find((c) => c.name === name.value))
const containerLeafCharts = computed(() => {
  const c = containerLeaf.value
  if (!c) return []
  return [
    { title: 'CPU Usage', sub: 'container %', unit: '%', series: [{ name: 'cpu', color: C.teal, data: c.series.cpu }] },
    { title: 'Memory', sub: 'RSS bytes', unit: 'B', series: [{ name: 'mem', color: C.teal, data: c.series.mem || [] }] },
    { title: 'Network', sub: 'rx+tx', unit: 'B/s', series: [{ name: 'net', color: C.blue, data: c.series.net || [] }] },
  ]
})
const clusterAgg = computed(() => {
  const ns = clusterNodes.value
  const a = (f) => { const v = ns.map(f).filter((x) => x != null); return v.length ? Math.round(v.reduce((x, y) => x + y, 0) / v.length) : null }
  return { online: ns.filter(online).length, total: ns.length, cpu: a((x) => x.cpu_percent), mem: a((x) => pct(x.mem_used, x.mem_total)), disk: a((x) => pct(x.disk_used, x.disk_total)) }
})

// k8s cluster detail, fleet-style: overlay this cluster's nodes (from /api/fleet,
// filtered to the cluster's node names) on one chart per metric
const fleet = ref(null)
async function loadClusterFleet() { try { fleet.value = await api.get(`/api/fleet?range=${range.value}`) } catch {} }
const clusterFleetData = computed(() => {
  const f = fleet.value
  if (!f || !f.t) return null
  const keep = new Set(clusterNodes.value.map((n) => n.name))
  const groups = ['cpu', 'mem', 'disk', 'net']
  const arrays = [], map = []
  groups.forEach((g) => (f[g] || []).filter((s) => keep.has(s.name)).forEach((s) => { arrays.push(s.data); map.push([g, s.name]) }))
  if (!map.length) return null
  const { t, arrays: na } = insertGaps(f.t, arrays)
  const out = { t, cpu: [], mem: [], disk: [], net: [] }
  map.forEach(([g, name], i) => out[g].push({ name, data: na[i] }))
  return out
})
const clusterFleetCharts = computed(() => {
  const f = clusterFleetData.value
  if (!f) return []
  return [
    { title: 'CPU', unit: '%', series: f.cpu },
    { title: 'Memory', unit: '%', series: f.mem },
    { title: 'Disk', unit: '%', series: f.disk },
    { title: 'Network', unit: 'B/s', series: f.net },
  ].filter((c) => c.series.length)
})

async function reload() {
  error.value = ''
  if (type.value === 'k8s') { await loadCluster(); await loadClusterFleet(); return }
  if (type.value === 'container') return loadContainers()
  await loadMetrics()
  if (type.value === 'docker') await loadContainers()
}
// Sub-hour ranges poll every 1s (feels realtime); larger ranges every 5s.
const live = computed(() => ['30m', '1h'].includes(range.value))
let timer = null
function restartTimer() { clearInterval(timer); timer = setInterval(reload, live.value ? 1000 : 5000) }
onMounted(() => { reload(); restartTimer() })
onBeforeUnmount(() => clearInterval(timer))
// reload only when the target or range changes — NOT when ?sel (metric selection)
// changes, which would otherwise blank the charts and cause a flash
watch(() => [route.params.id, type.value, range.value, name.value, parent.value].join('|'), () => { metrics.value = null; containersList.value = []; reload(); restartTimer() })
</script>

<template>
  <AppShell :title="name">
    <!-- breadcrumb -->
    <nav class="mb-4 flex flex-wrap items-center gap-1.5 text-sm text-muted">
      <RouterLink to="/" class="hover:text-accent">Systems</RouterLink>
      <span class="text-faint">›</span>
      <template v-if="parent && ptype">
        <RouterLink :to="kindHref(ptype)" class="hover:text-accent">{{ TYPE_LABEL[ptype] }}</RouterLink><span class="text-faint">›</span>
        <RouterLink :to="`/system/${encodeURIComponent(parent)}?type=${ptype}&name=${encodeURIComponent(parent)}`" class="hover:text-accent">{{ parent }}</RouterLink>
        <span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
      </template>
      <template v-else>
        <RouterLink :to="kindHref(type)" class="hover:text-accent">{{ TYPE_LABEL[type] }}</RouterLink><span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
      </template>
    </nav>

    <!-- header -->
    <div class="mb-5 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-line bg-surface p-5">
      <span class="flex items-center gap-2 text-sm"><span class="h-2.5 w-2.5 rounded-full bg-accent"></span><span class="font-semibold text-accent">Up</span></span>
      <span class="text-lg font-semibold text-fg">{{ name }}</span>
      <span class="rounded bg-accent/10 px-2 py-0.5 text-xs text-accent">{{ TYPE_LABEL[type] }}</span>
    </div>

    <!-- range (charts views) -->
    <div v-if="['node','host','container','docker','k8s'].includes(type)" class="mb-4 flex flex-wrap items-center gap-2">
      <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-sm">
        <button v-for="[rr] in RANGES" :key="rr" @click="setRange(rr)" class="rounded-md px-3 py-1" :class="range === rr ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ rr }}</button>
      </div>
      <span class="text-xs text-muted">Resolution <span class="rounded bg-surface2 px-1.5 py-0.5 text-fg">{{ resOf }}</span></span>
      <span v-if="live" class="ml-auto flex items-center gap-1.5 text-xs text-accent"><span class="h-1.5 w-1.5 animate-pulse rounded-full bg-accent"></span>Live</span>
      <span v-else class="ml-auto text-xs text-faint">auto-refresh 5s</span>
    </div>

    <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

    <!-- node / host: full charts -->
    <div v-if="['node','host'].includes(type)" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <div v-for="c in hostCharts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 flex items-start justify-between"><div><div class="text-sm font-medium text-fg">{{ c.title }}</div><div class="text-xs text-faint">{{ c.sub }}</div></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
        <UplotChart :time="gappedMetrics?.t || []" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :area="c.area !== false" :sync-key="'host:' + String(id)"
          :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
      </div>
    </div>

    <!-- docker: fleet-style overlay of the host + its containers -->
    <template v-else-if="type === 'docker'">
      <div class="mb-3 flex flex-wrap items-center gap-2">
        <SystemSearch :items="[]" @add="addToken" />
        <span v-for="(c, i) in chips" :key="c + i" class="flex items-center gap-1 rounded-full border border-line bg-surface2 py-0.5 pl-2 pr-1 text-xs text-fg">
          <span class="tabular-nums">{{ c }}</span>
          <button @click="removeChip(i)" title="Remove filter" class="grid h-4 w-4 place-items-center rounded-full text-faint hover:bg-red-500/15 hover:text-red-500"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </span>
        <button v-if="chips.length" @click="resetFilters" class="text-xs text-muted hover:text-accent">Reset</button>
        <RouterLink :to="`/system/${id}?type=host&name=${encodeURIComponent(name)}`" class="ml-auto flex items-center gap-1 rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-accent hover:border-accent/50">Host details <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></RouterLink>
      </div>

      <div v-if="dockerCharts && dockerCharts.charts.length" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div v-for="c in dockerCharts.charts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
          <div class="mb-2 flex items-start justify-between"><div><div class="text-sm font-medium text-fg">{{ c.title }} <span class="text-xs text-faint">{{ c.series.length }} series</span></div><div class="text-xs text-faint">{{ c.sub }}</div></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
          <UplotChart :time="dockerCharts.t" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :area="false" :legend-values-always="false" :sync-key="'docker:' + String(id)"
            :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
        </div>
      </div>
      <p v-else class="rounded-xl border border-line bg-surface p-4 text-sm text-muted">No data for the current filter. <button v-if="chips.length" @click="resetFilters" class="text-accent hover:underline">Reset</button></p>

      <div class="mt-5 overflow-hidden rounded-xl border border-line">
        <div class="flex items-center gap-2 border-b border-line bg-surface px-4 py-2.5"><h2 class="text-sm font-semibold text-fg">Containers</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ containersList.length }}</span></div>
        <table class="w-full min-w-[520px] text-sm"><thead class="border-b border-line bg-surface text-left text-xs uppercase tracking-wider text-muted"><tr><th class="px-4 py-2.5 font-medium">Container</th><th class="px-4 py-2.5 font-medium">CPU</th><th class="px-4 py-2.5 font-medium">Mem</th></tr></thead>
          <tbody>
            <tr v-for="c in containersList" :key="c.name" class="lm-row border-b border-line last:border-0">
              <td class="px-4 py-3"><RouterLink :to="`/system/${id}?type=container&name=${encodeURIComponent(c.name)}&parent=${encodeURIComponent(name)}&ptype=docker`" class="text-fg hover:text-accent">{{ c.name }}</RouterLink></td>
              <td class="px-4 py-3"><Gauge :v="c.cpu" /></td>
              <td class="px-4 py-3 tabular-nums text-muted">{{ c.mem != null ? (c.mem/1048576).toFixed(0)+' MB' : '—' }}</td>
            </tr>
            <tr v-if="!containersList.length"><td colspan="3" class="px-4 py-6 text-center text-muted">No container data</td></tr>
          </tbody>
        </table>
      </div>
    </template>

    <!-- k8s: overview + nodes -->
    <template v-else-if="type === 'k8s'">
      <div class="mb-5 grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface p-3.5"><div class="text-xs uppercase tracking-wider text-muted">Nodes ready</div><div class="mt-1 text-xl font-semibold text-fg">{{ clusterAgg.online }}<span class="text-base text-faint">/{{ clusterAgg.total }}</span></div></div>
        <div class="rounded-xl border border-line bg-surface p-3.5"><div class="text-xs uppercase tracking-wider text-muted">Avg CPU</div><div class="mt-1 text-xl font-semibold text-fg">{{ clusterAgg.cpu ?? '—' }}%</div></div>
        <div class="rounded-xl border border-line bg-surface p-3.5"><div class="text-xs uppercase tracking-wider text-muted">Avg memory</div><div class="mt-1 text-xl font-semibold text-fg">{{ clusterAgg.mem ?? '—' }}%</div></div>
        <div class="rounded-xl border border-line bg-surface p-3.5"><div class="text-xs uppercase tracking-wider text-muted">Avg disk</div><div class="mt-1 text-xl font-semibold text-fg">{{ clusterAgg.disk ?? '—' }}%</div></div>
      </div>

      <!-- cluster nodes, fleet-style: one line per node per metric -->
      <div v-if="clusterFleetCharts.length" class="mb-5 grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div v-for="c in clusterFleetCharts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
          <div class="mb-2 flex items-start justify-between"><div class="text-sm font-medium text-fg">{{ c.title }} <span class="text-xs text-faint">{{ c.series.length }} nodes</span></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
          <UplotChart :time="clusterFleetData?.t || []" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :area="false" :legend-values-always="false" :sync-key="'k8s:' + String(id)"
            :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
        </div>
      </div>

      <div class="overflow-hidden rounded-xl border border-line">
        <div class="flex items-center gap-2 border-b border-line bg-surface px-4 py-2.5"><h2 class="text-sm font-semibold text-fg">Nodes</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ clusterNodes.length }}</span></div>
        <table class="w-full min-w-[560px] text-sm"><thead class="border-b border-line bg-surface text-left text-xs uppercase tracking-wider text-muted"><tr><th class="px-4 py-2.5 font-medium">Node</th><th class="px-4 py-2.5 font-medium">Status</th><th class="px-4 py-2.5 font-medium">CPU</th><th class="px-4 py-2.5 font-medium">Mem</th><th class="px-4 py-2.5 font-medium">Disk</th></tr></thead>
          <tbody>
            <tr v-for="n in clusterNodes" :key="n.id" class="lm-row border-b border-line last:border-0">
              <td class="px-4 py-3"><RouterLink :to="`/system/${n.id}?type=node&name=${encodeURIComponent(n.name)}&parent=${encodeURIComponent(name)}&ptype=k8s`" class="text-fg hover:text-accent">{{ n.name }}</RouterLink></td>
              <td class="px-4 py-3 text-sm" :class="online(n)?'text-accent':'text-red-500'">{{ online(n)?'online':'offline' }}</td>
              <td class="px-4 py-3"><Gauge :v="Math.round(n.cpu_percent||0)" /></td>
              <td class="px-4 py-3"><Gauge :v="pct(n.mem_used,n.mem_total)" /></td>
              <td class="px-4 py-3"><Gauge :v="pct(n.disk_used,n.disk_total)" /></td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <!-- container: charts -->
    <div v-else-if="type === 'container'" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <div v-for="c in containerLeafCharts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 flex items-start justify-between"><div class="text-sm font-medium text-fg">{{ c.title }} <span class="text-xs text-faint">{{ c.sub }}</span></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
        <UplotChart :time="containersTime" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :sync-key="'ctr:' + String(id)"
          :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
      </div>
      <p v-if="!containerLeaf" class="text-sm text-muted">No data for this container.</p>
    </div>
  </AppShell>
</template>
