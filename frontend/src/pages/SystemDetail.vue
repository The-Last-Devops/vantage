<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import UplotChart from '../components/UplotChart.vue'
import Gauge from '../components/Gauge.vue'

const route = useRoute()
const router = useRouter()
const id = computed(() => route.params.id)
const type = computed(() => route.query.type || 'node')
const name = computed(() => route.query.name || id.value)
const parent = computed(() => route.query.parent || '')
const ptype = computed(() => route.query.ptype || '')
const TYPE_LABEL = { node: 'Node', host: 'Host', docker: 'Docker', k8s: 'Kubernetes', container: 'Container' }
const RANGES = [['30m', '1m'], ['1h', '1m'], ['3h', '1m'], ['6h', '5m'], ['12h', '5m'], ['24h', '15m']]
// range persisted in the URL so F5 keeps it
const range = computed(() => route.query.range || '30m')
const resOf = computed(() => RANGES.find(([r]) => r === range.value)?.[1] || '1m')
function setRange(r) { router.replace({ query: { ...route.query, range: r } }) }
const SPAN = { '30m': 1800, '1h': 3600, '3h': 10800, '6h': 21600, '12h': 43200, '24h': 86400 }
// charts always span the full selected window (blank where data is missing)
const spanSeconds = computed(() => SPAN[range.value] || 0)

// legend interaction: hover a metric → show only it in its chart; click → pin
// (multi), persisted in the URL (?sel). Selection is per-metric, applied to the
// chart that owns the metric.
const selectedMetrics = computed(() => (route.query.sel || '').split(',').filter(Boolean))
const hoverMetric = ref(null)
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

const hostCharts = computed(() => {
  const m = metrics.value
  if (!m) return []
  const charts = [
    { title: 'CPU Usage', sub: 'overall %', unit: '%', series: [{ name: 'CPU', color: C.teal, data: m.cpu }] },
  ]
  // Full CPU breakdown (only where the agent reports it — Linux /proc/stat).
  if (m.cpu_user && m.cpu_user.some((v) => v > 0)) {
    charts.push({
      title: 'CPU breakdown', sub: 'user / system / iowait / steal', unit: '%',
      series: [
        { name: 'user', color: C.teal, data: m.cpu_user },
        { name: 'system', color: C.amber, data: m.cpu_system },
        { name: 'iowait', color: C.blue, data: m.cpu_iowait },
        { name: 'steal', color: C.purple, data: m.cpu_steal },
      ],
    })
  }
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

async function reload() {
  error.value = ''
  if (type.value === 'k8s') return loadCluster()
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
// range lives in the URL → fullPath changes cover both range switch and navigation
watch(() => route.fullPath, () => { metrics.value = null; containersList.value = []; reload(); restartTimer() })
</script>

<template>
  <AppShell :title="name">
    <!-- breadcrumb -->
    <nav class="mb-4 flex flex-wrap items-center gap-1.5 text-sm text-muted">
      <RouterLink to="/" class="hover:text-accent">Systems</RouterLink>
      <span class="text-faint">›</span>
      <template v-if="parent && ptype">
        <span>{{ TYPE_LABEL[ptype] }}</span><span class="text-faint">›</span>
        <RouterLink :to="`/system/${encodeURIComponent(parent)}?type=${ptype}&name=${encodeURIComponent(parent)}`" class="hover:text-accent">{{ parent }}</RouterLink>
        <span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
      </template>
      <template v-else>
        <span>{{ TYPE_LABEL[type] }}</span><span class="text-faint">›</span><span class="text-fg">{{ name }}</span>
      </template>
    </nav>

    <!-- header -->
    <div class="mb-5 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-line bg-surface p-5">
      <span class="flex items-center gap-2 text-sm"><span class="h-2.5 w-2.5 rounded-full bg-accent"></span><span class="font-semibold text-accent">Up</span></span>
      <span class="text-lg font-semibold text-fg">{{ name }}</span>
      <span class="rounded bg-accent/10 px-2 py-0.5 text-xs text-accent">{{ TYPE_LABEL[type] }}</span>
    </div>

    <!-- range (charts views) -->
    <div v-if="['node','host','container'].includes(type)" class="mb-4 flex flex-wrap items-center gap-2">
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
        <div class="mb-2 flex items-start justify-between"><div><div class="text-sm font-medium text-fg">{{ c.title }}</div><div class="text-xs text-faint">{{ c.sub }}</div></div></div>
        <UplotChart :time="metrics?.t || []" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :sync-key="'host:' + String(id)"
          :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" />
      </div>
    </div>

    <!-- docker: host snapshot + containers -->
    <template v-else-if="type === 'docker'">
      <div class="mb-5 rounded-xl border border-line bg-surface p-4">
        <div class="mb-3 flex items-center justify-between">
          <h2 class="text-sm font-semibold text-fg">Host</h2>
          <RouterLink :to="`/system/${id}?type=host&name=${encodeURIComponent(name)}`" class="flex items-center gap-1 rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-accent hover:border-accent/50">View details <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></RouterLink>
        </div>
        <div v-if="snapshot" class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
          <div><div class="text-xs text-muted">CPU</div><div class="mt-1.5"><Gauge :v="snapshot.cpu" /></div></div>
          <div><div class="text-xs text-muted">Memory</div><div class="mt-1.5"><Gauge :v="snapshot.mem" /></div></div>
          <div><div class="text-xs text-muted">Disk</div><div class="mt-1.5"><Gauge :v="snapshot.disk" /></div></div>
        </div>
      </div>
      <div class="overflow-hidden rounded-xl border border-line">
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
        <div class="mb-2 text-sm font-medium text-fg">{{ c.title }} <span class="text-xs text-faint">{{ c.sub }}</span></div>
        <UplotChart :time="containersTime" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :sync-key="'ctr:' + String(id)"
          :focus-names="chartFocus(c.series)" :selected-names="selectedMetrics" @legend-hover="hoverMetric = $event" @legend-toggle="toggleMetric" />
      </div>
      <p v-if="!containerLeaf" class="text-sm text-muted">No data for this container.</p>
    </div>
  </AppShell>
</template>
