<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import UplotChart from '../components/UplotChart.vue'
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
// the host's kind for the header badge + "filter on fleet" links
const kind = computed(() => {
  if (type.value === 'container') return 'container'
  if (ptype.value === 'k8s') return 'k8s'
  if (type.value === 'host' || type.value === 'docker') return 'docker'
  return 'node'
})
const typeLabel = computed(() => TYPE_LABEL[kind.value])
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
// Up if the latest sample is fresh (within ~90s)
const statusUp = computed(() => {
  const t = metrics.value?.t || containersTime.value
  return !!(t && t.length && Date.now() / 1000 - t[t.length - 1] < 90)
})

async function reload() {
  error.value = ''
  // a k8s cluster is just the fleet filtered to its nodes → send there
  if (type.value === 'k8s') { router.replace({ path: '/', query: { q: `cluster:${name.value}` } }); return }
  if (type.value === 'container') return loadContainers()
  await loadMetrics()
  if (type.value === 'docker') await loadContainers()
}
// Sub-hour ranges poll every 1s (feels realtime); larger ranges every 5s.
const live = computed(() => ['30m', '1h'].includes(range.value))
let timer = null
function restartTimer() { clearInterval(timer); timer = setInterval(reload, live.value ? 1000 : 5000) }
// node metadata (namespace, kernel, cpu model…) — fetched once per target, not polled
const meta = ref(null)
async function loadMeta() { try { const all = await api.get('/api/systems'); meta.value = all.find((s) => s.id === id.value) || null } catch { meta.value = null } }
onMounted(() => { reload(); restartTimer(); loadMeta() })
onBeforeUnmount(() => clearInterval(timer))
// reload only when the target or range changes — NOT when ?sel (metric selection)
// changes, which would otherwise blank the charts and cause a flash
watch(() => [route.params.id, type.value, range.value, name.value, parent.value].join('|'), () => { metrics.value = null; containersList.value = []; reload(); restartTimer(); loadMeta() })
</script>

<template>
  <AppShell :title="name">
    <template #title-after>
      <!-- type + cluster: click to filter the fleet (Systems) page -->
      <RouterLink v-if="kind !== 'container'" :to="{ path: '/', query: { q: `kind:${kind}` } }" :title="`Filter kind:${kind}`" class="rounded bg-accent/10 px-2 py-0.5 text-xs text-accent hover:bg-accent/20">{{ typeLabel }}</RouterLink>
      <span v-else class="rounded bg-accent/10 px-2 py-0.5 text-xs text-accent">{{ typeLabel }}</span>
      <RouterLink v-if="ptype === 'k8s' && parent" :to="{ path: '/', query: { q: `cluster:${parent}` } }" :title="`Filter cluster:${parent}`" class="rounded bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">{{ parent }}</RouterLink>
      <RouterLink v-if="meta && meta.namespace" :to="{ path: '/', query: { q: `ns:${meta.namespace}` } }" :title="`Filter ns:${meta.namespace}`" class="rounded bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">{{ meta.namespace }}</RouterLink>
      <span class="flex items-center gap-1.5"><span class="h-2 w-2 rounded-full" :class="statusUp ? 'bg-accent' : 'bg-red-500'"></span><span class="text-xs font-medium" :class="statusUp ? 'text-accent' : 'text-red-500'">{{ statusUp ? 'Up' : 'Down' }}</span></span>
    </template>
    <!-- breadcrumb (simple: back to the fleet) -->
    <nav class="mb-4 flex items-center gap-1.5 text-sm text-muted">
      <RouterLink :to="{ path: '/', query: route.query.ns ? { ns: route.query.ns } : {} }" class="hover:text-accent">Systems</RouterLink>
      <span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
    </nav>

    <!-- node metadata -->
    <div v-if="meta && type !== 'container'" class="mb-4 flex flex-wrap items-center gap-x-6 gap-y-1.5 rounded-xl border border-line bg-surface px-4 py-2.5 text-xs">
      <span><span class="text-faint">Type</span> <span class="text-fg">{{ TYPE_LABEL[meta.kind] || meta.kind }}</span></span>
      <span v-if="meta.cluster"><span class="text-faint">Cluster</span> <span class="text-fg">{{ meta.cluster }}</span></span>
      <span><span class="text-faint">Namespace</span> <span class="text-fg">{{ meta.namespace }}</span></span>
      <span v-if="meta.hostname"><span class="text-faint">Hostname</span> <span class="text-fg">{{ meta.hostname }}</span></span>
      <span v-if="meta.cpu_model"><span class="text-faint">CPU</span> <span class="text-fg">{{ meta.cpu_model }}<template v-if="meta.cpu_cores"> · {{ meta.cpu_cores }} cores</template></span></span>
      <span v-if="meta.kernel"><span class="text-faint">Kernel</span> <span class="text-fg">{{ meta.kernel }}</span></span>
      <span v-if="meta.agent_version"><span class="text-faint">Agent</span> <span class="text-fg">v{{ meta.agent_version }}</span></span>
    </div>

    <!-- range (charts views) -->
    <div v-if="['node','host','container','docker'].includes(type)" class="mb-4 flex flex-wrap items-center gap-2">
      <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-sm">
        <button v-for="[rr] in RANGES" :key="rr" @click="setRange(rr)" class="rounded-md px-3 py-1" :class="range === rr ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ rr }}</button>
      </div>
      <span class="text-xs text-muted">Resolution <span class="rounded bg-surface2 px-1.5 py-0.5 text-fg">{{ resOf }}</span></span>
      <span v-if="live" class="ml-auto flex items-center gap-1.5 text-xs text-accent"><span class="h-1.5 w-1.5 animate-pulse rounded-full bg-accent"></span>Live</span>
      <span v-else class="ml-auto text-xs text-faint">auto-refresh 5s</span>
    </div>

    <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

    <!-- node / docker / k8s-node: the host's own charts -->
    <div v-if="['node','host','docker'].includes(type)" class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <div v-for="c in hostCharts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 flex items-start justify-between"><div><div class="text-sm font-medium text-fg">{{ c.title }}</div><div class="text-xs text-faint">{{ c.sub }}</div></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
        <UplotChart :time="gappedMetrics?.t || []" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :area="c.area !== false" :sync-key="'host:' + String(id)"
          :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" :view-range="viewRange" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" @cursor-time="chartTime = $event" @zoom="setZoom" />
      </div>
    </div>

    <!-- docker: its containers, as a separate table (not mixed into the host charts) -->
    <div v-if="type === 'docker'" class="mt-5 overflow-hidden rounded-xl border border-line">
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
