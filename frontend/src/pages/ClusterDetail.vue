<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import DataTable from '../components/DataTable.vue'
import UplotChart from '../components/UplotChart.vue'
import UiSelect from '../components/UiSelect.vue'
import { api } from '../lib/api'
import { minLoad } from '../lib/minLoad'

const route = useRoute()
const router = useRouter()
const id = computed(() => route.params.id)
const name = computed(() => route.query.name || id.value)

// ---- URL-persisted view state ----
// Capped at 7d: kube_container_stats keeps raw ~14d (no rollup ladder), so longer
// ranges would chart an empty/partial window.
const RANGES = ['30m', '1h', '6h', '24h', '7d']
const SPAN = { '30m': 1800, '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800 }
const range = computed(() => route.query.range || '1h')
const setRange = (r) => router.replace({ query: { ...route.query, range: r } })
const spanSeconds = computed(() => SPAN[range.value] || 3600)

const BY_OPTS = [
  { value: 'namespace', label: 'Namespace' },
  { value: 'workload', label: 'Workload' },
  { value: 'label', label: 'Label' },
]
const by = computed(() => route.query.by || 'namespace')
const setBy = (v) => router.replace({ query: { ...route.query, by: v, sel: undefined, selns: undefined } })
const labelKey = computed(() => route.query.lk || 'app')
const setLabelKey = (v) => router.replace({ query: { ...route.query, lk: v || undefined, sel: undefined, selns: undefined } })

// namespace scope (top selector) — scopes the whole page
const nsScope = computed(() => route.query.ns || '')
const setNs = (v) => router.replace({ query: { ...route.query, ns: v || undefined, sel: undefined, selns: undefined } })

// focused group (drill-down): charts show just it + its containers
const sel = computed(() => route.query.sel || '')
const selns = computed(() => route.query.selns || '')
function focusGroup(row) {
  const same = sel.value === row.group && selns.value === (row.namespace || '')
  router.replace({ query: { ...route.query, sel: same ? undefined : row.group, selns: same ? undefined : (row.namespace || undefined) } })
}
const clearFocus = () => router.replace({ query: { ...route.query, sel: undefined, selns: undefined } })
const focused = computed(() => !!sel.value)

// API filter params for the focused group
function focusFilter() {
  if (!sel.value) return {}
  if (by.value === 'namespace') return { ns: sel.value }
  if (by.value === 'workload') return { ns: selns.value || nsScope.value, workload: sel.value.split('/').slice(1).join('/') }
  if (by.value === 'label') return { lk: labelKey.value, lv: sel.value }
  return {}
}
const qstr = (obj) => new URLSearchParams(Object.fromEntries(Object.entries(obj).filter(([, v]) => v != null && v !== ''))).toString()

// ---- data ----
const loaded = ref(false)
const nsList = ref([])
const agg = ref({ as_of: null, by: 'namespace', groups: [] })
const seriesBy = ref({ t: [], truncated: false, groups: [] }) // overlay (no focus)
const seriesOne = ref({ t: [], cpu_millicores: [], mem_bytes: [] }) // focused single
const containers = ref([])
const summary = ref(null)
const err = ref('')

async function loadSummary() {
  try { summary.value = await api.get(`/api/systems/${id.value}/kube/summary`) } catch { summary.value = null }
}
async function loadNamespaces() {
  try {
    const r = await api.get(`/api/systems/${id.value}/kube/aggregate?by=namespace`)
    nsList.value = r.groups.map((g) => g.group).filter(Boolean).sort()
  } catch { nsList.value = [] }
}
const nsOptions = computed(() => [{ value: '', label: 'All namespaces' }, ...nsList.value.map((n) => ({ value: n, label: n }))])

async function loadAgg() {
  const p = { by: by.value, ns: nsScope.value }
  if (by.value === 'label') p.label = labelKey.value
  agg.value = await api.get(`/api/systems/${id.value}/kube/aggregate?${qstr(p)}`)
}
async function loadCharts() {
  if (sel.value) {
    seriesOne.value = await api.get(`/api/systems/${id.value}/kube/series?${qstr({ range: range.value, ...focusFilter() })}`)
  } else {
    const p = { by: by.value, ns: nsScope.value, range: range.value }
    if (by.value === 'label') p.label = labelKey.value
    seriesBy.value = await api.get(`/api/systems/${id.value}/kube/series-by?${qstr(p)}`)
  }
}
async function loadContainers() {
  if (!sel.value) { containers.value = []; return }
  containers.value = await api.get(`/api/systems/${id.value}/kube/containers?${qstr(focusFilter())}`)
}

async function reloadAll(first = false) {
  err.value = ''
  try {
    const tasks = [loadAgg(), loadCharts(), loadContainers(), loadSummary()]
    if (first) tasks.push(loadNamespaces())
    await (first ? minLoad(Promise.all(tasks)) : Promise.all(tasks))
  } catch (e) {
    err.value = `Failed to load cluster stats (${e.status || '?'}).`
  } finally {
    if (first) loaded.value = true
  }
}

let timer = null
const liveRange = computed(() => ['30m', '1h'].includes(range.value))
function restartTimer() { clearInterval(timer); timer = setInterval(() => reloadAll(false), liveRange.value ? 3000 : 8000) }
onMounted(() => { reloadAll(true); restartTimer() })
onBeforeUnmount(() => clearInterval(timer))
watch(() => [id.value, range.value, by.value, labelKey.value, nsScope.value, sel.value, selns.value].join('|'), () => { reloadAll(false); restartTimer() })

// ---- formatting ----
const fmtCores = (mc) => (mc / 1000).toFixed(mc < 100 ? 3 : mc < 100000 ? 2 : 1)
function fmtBytes(b) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0, n = Number(b) || 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${u[i]}`
}
const color = (i) => `hsl(${(i * 47) % 360} 70% 58%)`
// CPU bar width is relative to the busiest group in the current view; colour by that
// relative magnitude (top usage = down, high = warn, else accent) for quick scanning.
const maxCpu = computed(() => Math.max(1, ...agg.value.groups.map((g) => g.cpu_millicores || 0)))
const barPct = (mc) => Math.round(((mc || 0) / maxCpu.value) * 100)
const barColor = (mc) => { const p = barPct(mc); return p >= 90 ? 'rgb(var(--down))' : p >= 70 ? 'rgb(var(--warn))' : 'rgb(var(--accent))' }
// a "Kind/name" workload group → the kind chip + the name
const splitKind = (g) => { const i = (g || '').indexOf('/'); return i > 0 ? { kind: g.slice(0, i), name: g.slice(i + 1) } : { kind: '', name: g || '—' } }
const kCores = (mc) => (mc / 1000).toFixed(mc >= 100000 ? 0 : 1)

// ---- charts (overlay per-group when not focused; single line when focused) ----
const chartTime = computed(() => (focused.value ? seriesOne.value.t : seriesBy.value.t) || [])
const cpuSeries = computed(() => {
  if (focused.value) return [{ name: 'CPU', color: '#6ee7b7', data: (seriesOne.value.cpu_millicores || []).map((v) => v / 1000) }]
  return (seriesBy.value.groups || []).map((g, i) => ({ name: g.name, color: color(i), data: g.cpu_millicores.map((v) => (v == null ? null : v / 1000)) }))
})
const memSeries = computed(() => {
  if (focused.value) return [{ name: 'Memory', color: '#7aa2f7', data: seriesOne.value.mem_bytes || [] }]
  return (seriesBy.value.groups || []).map((g, i) => ({ name: g.name, color: color(i), data: g.mem_bytes }))
})

// ---- table ----
const groupHeader = computed(() => (by.value === 'label' ? `Label: ${labelKey.value}` : by.value === 'workload' ? 'Workload' : 'Namespace'))
const showNsCol = computed(() => by.value === 'workload')
const aggCols = computed(() => [
  ...(showNsCol.value ? [{ key: 'namespace', label: 'Namespace', sortable: true }] : []),
  { key: 'group', label: groupHeader.value, sortable: true },
  { key: 'cpu_millicores', label: 'CPU', sortable: true, align: 'right', mono: true },
  { key: 'mem_bytes', label: 'Memory', sortable: true, align: 'right', mono: true },
  { key: 'pods', label: 'Pods', sortable: true, align: 'right', mono: true },
  { key: 'containers', label: 'Containers', sortable: true, align: 'right', mono: true },
  { key: 'restarts', label: 'Restarts', sortable: true, align: 'right', mono: true },
])
const ctrCols = [
  { key: 'pod', label: 'Pod', sortable: true },
  { key: 'container', label: 'Container', sortable: true },
  { key: 'node', label: 'Node', sortable: true },
  { key: 'phase', label: 'Phase', sortable: true },
  { key: 'cpu_millicores', label: 'CPU', sortable: true, align: 'right', mono: true },
  { key: 'mem_bytes', label: 'Memory', sortable: true, align: 'right', mono: true },
  { key: 'restarts', label: 'Restarts', sortable: true, align: 'right', mono: true },
]
const rowKey = (r) => (r.namespace ? r.namespace + '/' : '') + (r.group ?? '—')
const isFocusedRow = (r) => sel.value === r.group && selns.value === (r.namespace || '')

const asOf = computed(() => (agg.value.as_of ? new Date(agg.value.as_of * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : null))
const noMetrics = computed(() => agg.value.groups.length > 0 && agg.value.groups.every((g) => !g.cpu_millicores && !g.mem_bytes) && agg.value.groups.some((g) => g.containers > 0))
const scopeLabel = computed(() => (sel.value ? `${by.value === 'label' ? labelKey.value + '=' : ''}${sel.value}` : nsScope.value || 'whole cluster'))
</script>

<template>
  <AppShell :title="name" :breadcrumb="[{ label: 'Clusters', to: { name: 'clusters', query: route.query.ws ? { ws: route.query.ws } : {} } }, { label: name }]">
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <p v-if="err" class="rounded-lg border border-down/40 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>
      <p v-if="noMetrics" class="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-xs text-warn">
        metrics-server not detected in this cluster — CPU/memory read 0. Install metrics-server to populate usage (metadata is still collected).
      </p>

      <!-- KPI strip: cluster roll-up -->
      <div v-if="summary" class="grid grid-cols-2 gap-px overflow-hidden rounded-2xl border border-line bg-line sm:grid-cols-5">
        <div class="bg-surface px-4 py-2.5">
          <div class="text-[10px] font-bold uppercase tracking-wide text-faint">CPU used</div>
          <div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-accent">{{ kCores(summary.cpu_millicores) }}<span class="text-xs font-semibold text-faint"> cores</span></div>
        </div>
        <div class="bg-surface px-4 py-2.5">
          <div class="text-[10px] font-bold uppercase tracking-wide text-faint">Memory used</div>
          <div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ fmtBytes(summary.mem_bytes) }}</div>
        </div>
        <div class="bg-surface px-4 py-2.5">
          <div class="text-[10px] font-bold uppercase tracking-wide text-faint">Pods running</div>
          <div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ summary.pods_running }}<span class="text-xs font-semibold text-faint"> / {{ summary.pods }}</span></div>
        </div>
        <div class="bg-surface px-4 py-2.5">
          <div class="text-[10px] font-bold uppercase tracking-wide text-faint">Containers</div>
          <div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ summary.containers }}</div>
        </div>
        <div class="bg-surface px-4 py-2.5">
          <div class="text-[10px] font-bold uppercase tracking-wide text-faint">Restarts</div>
          <div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums" :class="summary.restarts > 0 ? 'text-warn' : 'text-fg'">{{ summary.restarts }}</div>
        </div>
      </div>

      <!-- charts -->
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="mr-auto text-sm font-semibold text-fg">
          Usage · <span class="text-muted">{{ scopeLabel }}</span>
          <button v-if="focused" @click="clearFocus" class="ml-2 rounded border border-line px-2 py-0.5 text-xs text-muted hover:text-fg">← all {{ by }}s</button>
          <span v-else-if="seriesBy.groups.length" class="ml-2 text-xs font-normal text-faint">overlay of {{ seriesBy.groups.length }} {{ by }}{{ seriesBy.groups.length > 1 ? 's' : '' }}<template v-if="seriesBy.truncated"> (top {{ seriesBy.groups.length }})</template> · hover a line for its name</span>
        </h2>
        <div class="flex gap-1 rounded-lg border border-line bg-surface2 p-0.5">
          <button v-for="r in RANGES" :key="r" @click="setRange(r)"
            class="rounded px-2.5 py-1 text-xs font-medium" :class="range === r ? 'bg-accent text-accentfg' : 'text-muted hover:text-fg'">{{ r }}</button>
        </div>
      </div>
      <div class="grid gap-3.5 md:grid-cols-2">
        <div class="rounded-2xl border border-line bg-surface p-3">
          <div class="mb-1 text-xs font-semibold text-muted">CPU (cores)</div>
          <UplotChart :time="chartTime" :series="cpuSeries" unit="" :span-seconds="spanSeconds" :area="focused" sync-key="kube-cluster" />
        </div>
        <div class="rounded-2xl border border-line bg-surface p-3">
          <div class="mb-1 text-xs font-semibold text-muted">Memory</div>
          <UplotChart :time="chartTime" :series="memSeries" unit="B" :span-seconds="spanSeconds" :area="focused" sync-key="kube-cluster" />
        </div>
      </div>

      <!-- grouping controls -->
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="mr-auto text-sm font-semibold text-fg">Breakdown</h2>
        <span class="text-xs text-faint">namespace</span>
        <UiSelect :model-value="nsScope" @update:model-value="setNs" :options="nsOptions" />
        <span class="text-xs text-faint">group by</span>
        <div class="flex gap-1 rounded-lg border border-line bg-surface2 p-0.5">
          <button v-for="o in BY_OPTS" :key="o.value" @click="setBy(o.value)"
            class="rounded px-2.5 py-1 text-xs font-medium" :class="by === o.value ? 'bg-accent text-accentfg' : 'text-muted hover:text-fg'">{{ o.label }}</button>
        </div>
        <input v-if="by === 'label'" :value="labelKey" @change="setLabelKey($event.target.value.trim())"
          placeholder="label key" class="w-36 rounded-lg border border-line bg-surface2 px-2.5 py-1.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        <span v-if="asOf" class="text-xs text-faint">as of {{ asOf }}</span>
      </div>

      <DataTable :columns="aggCols" :rows="agg.groups" :row-key="rowKey" clickable
        :filterable="agg.groups.length > 8" filter-placeholder="Filter groups…"
        :initial-sort="{ key: 'cpu_millicores', dir: 'desc' }"
        empty="No pods reported yet — is the k8s-cluster agent deployed?"
        @row-click="focusGroup">
        <template #cell-group="{ row }">
          <div class="flex min-w-0 items-center gap-2">
            <span v-if="by === 'workload' && splitKind(row.group).kind" class="shrink-0 rounded border border-line bg-surface2 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-faint">{{ splitKind(row.group).kind }}</span>
            <span class="truncate font-mono text-sm" :class="isFocusedRow(row) ? 'text-accent' : 'text-fg'">{{ by === 'workload' ? splitKind(row.group).name : (row.group ?? '—') }}</span>
          </div>
        </template>
        <template #cell-cpu_millicores="{ row }">
          <div class="flex items-center justify-end gap-2">
            <span class="hidden h-1.5 w-14 shrink-0 overflow-hidden rounded-full sm:inline-block" style="background:rgb(var(--track))"><span class="block h-full rounded-full" :style="{ width: barPct(row.cpu_millicores) + '%', background: barColor(row.cpu_millicores) }"></span></span>
            <span class="font-mono tabular-nums">{{ fmtCores(row.cpu_millicores) }}</span>
          </div>
        </template>
        <template #cell-mem_bytes="{ row }">{{ fmtBytes(row.mem_bytes) }}</template>
      </DataTable>

      <!-- drill-down: containers of the focused group -->
      <template v-if="focused">
        <h2 class="text-sm font-semibold text-fg">Containers · <span class="text-muted">{{ scopeLabel }}</span></h2>
        <DataTable :columns="ctrCols" :rows="containers" :row-key="(r) => r.namespace + '/' + r.pod + '/' + r.container"
          :filterable="containers.length > 8" filter-placeholder="Filter containers…"
          :initial-sort="{ key: 'cpu_millicores', dir: 'desc' }" empty="No containers.">
          <template #cell-cpu_millicores="{ row }">{{ fmtCores(row.cpu_millicores) }}</template>
          <template #cell-mem_bytes="{ row }">{{ fmtBytes(row.mem_bytes) }}</template>
          <template #cell-phase="{ row }">
            <span class="rounded px-1.5 py-0.5 text-xs" :class="row.phase === 'Running' ? 'bg-accent/12 text-accent' : row.phase === 'Failed' ? 'bg-down/12 text-down' : 'bg-surface2 text-muted'">{{ row.phase || '—' }}</span>
          </template>
        </DataTable>
      </template>
    </div>
  </AppShell>
</template>
