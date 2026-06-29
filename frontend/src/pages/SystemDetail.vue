<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { minLoad } from '../lib/minLoad'
import FleetCharts from '../components/FleetCharts.vue'
import Gauge from '../components/Gauge.vue'
import SystemMetaCard from '../components/SystemMetaCard.vue'
import SystemAlertRules from '../components/SystemAlertRules.vue'
import SystemShellCard from '../components/SystemShellCard.vue'
import SystemRangePicker from '../components/SystemRangePicker.vue'
import SystemChartPanel from '../components/SystemChartPanel.vue'
import { encodeZoom, decodeZoom } from '../lib/zoom'
import { insertGaps } from '../lib/gaps'

const route = useRoute()
const router = useRouter()
const id = computed(() => route.params.id)
const type = computed(() => route.query.type || 'node')
const name = computed(() => route.query.name || meta.value?.name || id.value)
const parent = computed(() => route.query.parent || '')
const ptype = computed(() => route.query.ptype || '')
const TYPE_LABEL = { node: 'Node', host: 'Host', docker: 'Docker', k8s: 'Kubernetes', container: 'Container' }
// the host's kind for the header badge + "filter on fleet" links
const kind = computed(() => {
  if (type.value === 'container') return 'container'
  if (ptype.value === 'k8s') return 'k8s'
  if (['host', 'docker', 'containers'].includes(type.value)) return 'docker'
  return 'node'
})
const typeLabel = computed(() => TYPE_LABEL[kind.value])
const RANGES = [['30m', '1m'], ['1h', '1m'], ['3h', '2m'], ['6h', '5m'], ['12h', '10m'], ['24h', '15m'], ['7d', '1h'], ['30d', '6h'], ['90d', '1d'], ['1y', '1d']]
// range persisted in the URL so F5 keeps it
const range = computed(() => route.query.range || '30m')
const resOf = computed(() => RANGES.find(([r]) => r === range.value)?.[1] || '1m')
function setRange(r) { router.replace({ query: { ...route.query, range: r, zoom: undefined } }) }
// drag-zoom window persisted in the URL as a human-readable range, shared by all charts
const viewRange = computed(() => decodeZoom(route.query.zoom))
function setZoom(r) { router.replace({ query: { ...route.query, zoom: encodeZoom(r) } }) }
const SPAN = { '30m': 1800, '1h': 3600, '3h': 10800, '6h': 21600, '12h': 43200, '24h': 86400, '7d': 604800, '30d': 2592000, '90d': 7776000, '1y': 31536000 }
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
// single focus for the containers fleet grid (hover/pin a container)
const fleetFocus = computed(() => (hoverMetric.value ? [hoverMetric.value] : selectedMetrics.value.length ? selectedMetrics.value : null))
const seriesColor = (i) => `hsl(${(i * 47) % 360} 70% 58%)`
const fmtBps = (v) => {
  if (v == null) return '—'
  const u = ['B', 'K', 'M', 'G']; let i = 0; let n = v
  while (n >= 1024 && i < 3) { n /= 1024; i++ }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${u[i]}/s`
}

const metrics = ref(null)
const containersList = ref([])
const containersTime = ref([])
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
  const keys = ['cpu', 'mem_pct', 'disk_pct', 'net_rx', 'net_tx', 'dr', 'dw', 'load1', 'load5', 'load15', 'cpu_user', 'cpu_system', 'cpu_iowait', 'cpu_steal', 'disk_util'].filter((k) => Array.isArray(m[k]))
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
    { title: 'Disk utilization', sub: 'busiest disk % busy', unit: '%', series: [{ name: 'util', color: C.purple, data: m.disk_util || [] }] },
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
// containers fleet (type=containers): overlay this host's containers per metric
const containersFleet = computed(() => {
  const list = containersList.value, ct = containersTime.value
  if (!list.length || ct.length < 3) return null
  const mk = (key) => list.map((c, i) => ({ name: c.name, color: seriesColor(i), data: c.series[key] })).filter((s) => Array.isArray(s.data))
  const groups = { cpu: mk('cpu'), mem: mk('mem'), net: mk('net') }
  const arrays = [], map = []
  ;['cpu', 'mem', 'net'].forEach((g) => groups[g].forEach((s) => { arrays.push(s.data); map.push([g, s.name, s.color]) }))
  const { t, arrays: na } = insertGaps(ct, arrays)
  const out = { cpu: [], mem: [], net: [] }
  map.forEach(([g, name, color], i) => out[g].push({ name, color, data: na[i] }))
  return {
    t,
    charts: [
      { title: 'CPU', unit: '%', series: out.cpu },
      { title: 'Memory', unit: 'B', series: out.mem },
      { title: 'Network', unit: 'B/s', series: out.net },
    ].filter((c) => c.series.length),
  }
})
// Tri-state so a page reload doesn't flash "Down" before data arrives:
//   null = unknown (no data fetched yet), true = Up, false = Down.
const statusUp = computed(() => {
  const t = metrics.value?.t || containersTime.value
  if (!t || !t.length) return null
  return Date.now() / 1000 - t[t.length - 1] < 90
})
const statusLabel = computed(() => (statusUp.value === null ? 'Checking…' : statusUp.value ? 'Up' : 'Down'))
const statusText = computed(() => (statusUp.value === null ? 'text-muted' : statusUp.value ? 'text-accent' : 'text-red-500'))
const statusDot = computed(() => (statusUp.value === null ? 'bg-faint animate-pulse' : statusUp.value ? 'bg-accent' : 'bg-red-500'))

async function reload() {
  error.value = ''
  // a k8s cluster is just the fleet filtered to its nodes → send there
  if (type.value === 'k8s') { router.replace({ path: '/', query: { q: `cluster:${name.value}` } }); return }
  if (type.value === 'container' || type.value === 'containers') return loadContainers()
  await loadMetrics()
  if (type.value === 'docker') await loadContainers()
}
// Sub-hour ranges poll every 1s (feels realtime); larger ranges every 5s.
const live = computed(() => ['30m', '1h'].includes(range.value))
let timer = null
function restartTimer() { clearInterval(timer); timer = setInterval(reload, live.value ? 1000 : 5000) }
// node metadata (namespace, kernel, cpu model…) — fetched once per target, not polled
const meta = ref(null)
async function loadMeta() { try { const all = await minLoad(api.get('/api/systems')); meta.value = all.find((s) => s.id === id.value) || null } catch { meta.value = null } }
const rules = ref([]) // alert rules covering this host (own + namespace-wide)
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
async function loadRules() { try { rules.value = await api.get(`/api/systems/${id.value}/alerts`) } catch { rules.value = [] } }
onMounted(() => { reload(); restartTimer(); loadMeta(); loadRules() })
onBeforeUnmount(() => clearInterval(timer))
// reload only when the target or range changes — NOT when ?sel (metric selection)
// changes, which would otherwise blank the charts and cause a flash
watch(() => [route.params.id, type.value, range.value, name.value, parent.value].join('|'), () => { metrics.value = null; containersList.value = []; reload(); restartTimer(); loadMeta(); loadRules() })
</script>

<template>
  <AppShell :title="name" :breadcrumb="[{ label: 'Infrastructure', to: { name: 'systems', query: route.query.ns ? { ns: route.query.ns } : {} } }, { label: name }]">
    <!-- breadcrumb sits in the top bar (left); status on the right -->
    <template #title-after>
      <nav class="flex items-center gap-1.5 text-base font-semibold">
        <RouterLink :to="{ path: '/', query: route.query.ns ? { ns: route.query.ns } : {} }" class="text-muted hover:text-accent">Infrastructure</RouterLink>
        <span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
      </nav>
    </template>
    <template #header>
      <span class="flex items-center gap-1.5"><span class="h-2 w-2 rounded-full" :class="statusDot"></span><span class="text-xs font-medium" :class="statusText">{{ statusLabel }}</span></span>
    </template>

    <!-- first paint while metadata loads — never a blank content area -->
    <PageLoader v-if="!meta" />

    <!-- node / container metadata — every field links to the fleet filtered by that value -->
    <SystemMetaCard v-if="meta" :meta="meta" :type="type" :id="id" />

    <!-- alert rules covering this host -->
    <SystemAlertRules v-if="meta && type !== 'container'" :rules="rules" :nsq="nsq" />

    <!-- shell / SSH console settings (hosts/nodes only) -->
    <SystemShellCard v-if="meta && ['node','host','docker'].includes(type)" :id="id" :name="name" />

    <!-- range (charts views) -->
    <SystemRangePicker v-if="['node','host','container','docker','containers'].includes(type)" :ranges="RANGES" :range="range" :res-of="resOf" :live="live" @set-range="setRange" />

    <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

    <!-- node / docker / k8s-node: the host's own charts -->
    <div v-if="['node','host','docker'].includes(type)" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <SystemChartPanel v-for="c in hostCharts" :key="c.title" :chart="c" :time="gappedMetrics?.t || []" :header-time="headerTime" :span-seconds="spanSeconds" :sync-key="'host:' + String(id)"
        :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
    </div>

    <!-- docker: its containers, as a separate table (link to the fleet view) -->
    <div v-if="type === 'docker'" class="mt-5 overflow-hidden rounded-xl border border-line">
      <div class="flex items-center gap-2 border-b border-line bg-surface px-4 py-2.5">
        <RouterLink :to="`/system/${id}?type=containers&name=${encodeURIComponent(name)}`" class="text-sm font-semibold text-fg hover:text-accent" v-tip="`View containers as a fleet`">Containers</RouterLink><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ containersList.length }}</span>
      </div>
      <table class="w-full min-w-[600px] text-sm"><thead class="border-b border-line bg-surface text-left text-xs uppercase tracking-wider text-muted"><tr><th class="px-4 py-2.5 font-medium">Container</th><th class="px-4 py-2.5 font-medium">CPU</th><th class="px-4 py-2.5 font-medium">Mem</th><th class="px-4 py-2.5 font-medium">Network</th></tr></thead>
        <tbody>
          <tr v-for="c in containersList" :key="c.name" class="vantage-row border-b border-line last:border-0">
            <td class="px-4 py-3"><RouterLink :to="`/system/${id}?type=container&name=${encodeURIComponent(c.name)}&parent=${encodeURIComponent(name)}&ptype=docker`" class="text-fg hover:text-accent">{{ c.name }}</RouterLink></td>
            <td class="px-4 py-3"><Gauge :v="c.cpu" /></td>
            <td class="px-4 py-3 tabular-nums text-muted">{{ c.mem != null ? (c.mem/1048576).toFixed(0)+' MB' : '—' }}</td>
            <td class="px-4 py-3 tabular-nums text-muted">{{ fmtBps(c.net) }}</td>
          </tr>
          <tr v-if="!containersList.length"><td colspan="4" class="px-4 py-6 text-center text-muted">No container data</td></tr>
        </tbody>
      </table>
    </div>

    <!-- containers fleet: overlay this host's containers + table-as-selector -->
    <template v-else-if="type === 'containers'">
      <FleetCharts v-if="containersFleet" :charts="containersFleet.charts" :time="containersFleet.t" :span-seconds="spanSeconds" :view-range="viewRange"
        :focus-names="fleetFocus" :selected-names="selectedMetrics" :sync-key="'ctrs:' + String(id)"
        @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @zoom="setZoom" />
      <p v-else class="rounded-xl border border-line bg-surface p-4 text-sm text-muted">No container data.</p>

      <div class="mt-5 overflow-hidden rounded-xl border border-line">
        <div class="flex items-center gap-2 border-b border-line bg-surface px-4 py-2.5"><h2 class="text-sm font-semibold text-fg">Containers</h2><span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ containersList.length }}</span></div>
        <table class="w-full min-w-[600px] text-sm"><thead class="border-b border-line bg-surface text-left text-xs uppercase tracking-wider text-muted"><tr><th class="px-4 py-2.5 font-medium">Container</th><th class="px-4 py-2.5 font-medium">CPU</th><th class="px-4 py-2.5 font-medium">Mem</th><th class="px-4 py-2.5 font-medium">Network</th></tr></thead>
          <tbody>
            <tr v-for="(c, i) in containersList" :key="c.name" class="vantage-row border-b border-line last:border-0" :class="selectedMetrics.includes(c.name) ? 'sel' : ''" @mouseenter="hoverMetric = c.name" @mouseleave="hoverMetric = null">
              <td class="px-4 py-3"><div class="flex items-center gap-2"><button @click="toggleMetric(c.name)" v-tip="selectedMetrics.includes(c.name) ? 'Unpin' : 'Pin on charts'" class="h-2.5 w-2.5 shrink-0 rounded-full" :class="selectedMetrics.includes(c.name) ? 'ring-2 ring-offset-1 ring-offset-surface' : ''" :style="{ background: seriesColor(i), '--tw-ring-color': seriesColor(i) }"></button><span class="text-fg">{{ c.name }}</span></div></td>
              <td class="px-4 py-3"><Gauge :v="c.cpu" /></td>
              <td class="px-4 py-3 tabular-nums text-muted">{{ c.mem != null ? (c.mem/1048576).toFixed(0)+' MB' : '—' }}</td>
              <td class="px-4 py-3 tabular-nums text-muted">{{ fmtBps(c.net) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <!-- container: charts -->
    <div v-else-if="type === 'container'" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <SystemChartPanel v-for="c in containerLeafCharts" :key="c.title" :chart="c" :time="containersTime" :header-time="headerTime" :span-seconds="spanSeconds" :sync-key="'ctr:' + String(id)" inline-sub
        :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
      <p v-if="!containerLeaf" class="text-sm text-muted">No data for this container.</p>
    </div>
  </AppShell>
</template>
