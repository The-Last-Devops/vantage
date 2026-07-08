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

// --- URL-persisted view state ---
const RANGES = ['30m', '1h', '6h', '24h', '7d', '30d']
const range = computed(() => route.query.range || '1h')
const setRange = (r) => router.replace({ query: { ...route.query, range: r } })

const BY_OPTS = [
  { value: 'namespace', label: 'Namespace' },
  { value: 'workload', label: 'Workload' },
  { value: 'label', label: 'Label' },
]
const by = computed(() => route.query.by || 'namespace')
const setBy = (v) => router.replace({ query: { ...route.query, by: v, sel: undefined } })
const labelKey = computed(() => route.query.lk || 'app')
const setLabelKey = (v) => router.replace({ query: { ...route.query, lk: v || undefined, sel: undefined } })

// selected group for drill-down (charts + container list scope to it)
const sel = computed(() => route.query.sel || '')
const selectGroup = (v) => router.replace({ query: { ...route.query, sel: sel.value === v ? undefined : v } })

// --- data ---
const loaded = ref(false)
const agg = ref({ as_of: null, by: 'namespace', groups: [] })
const series = ref({ t: [], cpu_millicores: [], mem_bytes: [] })
const containers = ref([])
const err = ref('')

// Translate the selected group into API filter params for series + container list.
function selFilter() {
  if (!sel.value) return {}
  if (by.value === 'namespace') return { ns: sel.value }
  if (by.value === 'workload') return { workload: sel.value.split('/').slice(1).join('/') }
  if (by.value === 'label') return { lk: labelKey.value, lv: sel.value }
  return {}
}
const extra = (obj) => {
  const s = new URLSearchParams(obj).toString()
  return s ? `&${s}` : ''
}

async function loadAgg() {
  const lbl = by.value === 'label' ? `&label=${encodeURIComponent(labelKey.value)}` : ''
  agg.value = await api.get(`/api/systems/${id.value}/kube/aggregate?by=${by.value}${lbl}`)
}
async function loadSeries() {
  series.value = await api.get(`/api/systems/${id.value}/kube/series?range=${range.value}${extra(selFilter())}`)
}
async function loadContainers() {
  if (!sel.value) { containers.value = []; return }
  const s = new URLSearchParams(selFilter()).toString()
  containers.value = await api.get(`/api/systems/${id.value}/kube/containers?${s}`)
}

async function reloadAll(first = false) {
  err.value = ''
  try {
    const p = Promise.all([loadAgg(), loadSeries(), loadContainers()])
    await (first ? minLoad(p) : p)
  } catch (e) {
    err.value = `Failed to load cluster stats (${e.status || '?'}).`
  } finally {
    if (first) loaded.value = true
  }
}

let timer = null
const live = computed(() => ['30m', '1h'].includes(range.value))
function restartTimer() { clearInterval(timer); timer = setInterval(() => reloadAll(false), live.value ? 3000 : 8000) }
onMounted(() => { reloadAll(true); restartTimer() })
onBeforeUnmount(() => clearInterval(timer))
watch(() => [id.value, range.value, by.value, labelKey.value, sel.value].join('|'), () => { reloadAll(false); restartTimer() })

// --- formatting ---
const fmtCores = (mc) => (mc / 1000).toFixed(mc < 100 ? 3 : mc < 100000 ? 2 : 1)
function fmtBytes(b) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0, n = Number(b) || 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${u[i]}`
}

const groupHeader = computed(() => (by.value === 'label' ? `Label: ${labelKey.value}` : by.value === 'workload' ? 'Workload' : 'Namespace'))
const aggCols = computed(() => [
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

const cpuChart = computed(() => ({ time: series.value.t || [], series: [{ name: 'CPU (cores)', color: '#6ee7b7', data: (series.value.cpu_millicores || []).map((v) => v / 1000) }] }))
const memChart = computed(() => ({ time: series.value.t || [], series: [{ name: 'Memory', color: '#7aa2f7', data: series.value.mem_bytes || [] }] }))

const asOf = computed(() => (agg.value.as_of ? new Date(agg.value.as_of * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : null))
// metrics-server missing → containers exist but every usage reads 0
const noMetrics = computed(() => agg.value.groups.length > 0 && agg.value.groups.every((g) => !g.cpu_millicores && !g.mem_bytes) && agg.value.groups.some((g) => g.containers > 0))
const scopeLabel = computed(() => (sel.value ? `${by.value === 'label' ? labelKey.value + '=' : ''}${sel.value}` : 'whole cluster'))
</script>

<template>
  <AppShell :title="name" :breadcrumb="[{ label: 'Infrastructure', to: { name: 'systems', query: route.query.ws ? { ws: route.query.ws } : {} } }, { label: name }]">
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <p v-if="err" class="rounded-lg border border-down/40 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>
      <p v-if="noMetrics" class="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-xs text-warn">
        metrics-server not detected in this cluster — CPU/memory read 0. Install metrics-server to populate usage (metadata is still collected).
      </p>

      <!-- charts: total usage for the current scope -->
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="mr-auto text-sm font-semibold text-fg">Usage · <span class="text-muted">{{ scopeLabel }}</span></h2>
        <div class="flex gap-1 rounded-lg border border-line bg-surface2 p-0.5">
          <button v-for="r in RANGES" :key="r" @click="setRange(r)"
            class="rounded px-2.5 py-1 text-xs font-medium" :class="range === r ? 'bg-accent text-accentfg' : 'text-muted hover:text-fg'">{{ r }}</button>
        </div>
      </div>
      <div class="grid gap-3.5 md:grid-cols-2">
        <div class="rounded-2xl border border-line bg-surface p-3">
          <div class="mb-1 text-xs font-semibold text-muted">CPU (cores)</div>
          <UplotChart :time="cpuChart.time" :series="cpuChart.series" unit="" />
        </div>
        <div class="rounded-2xl border border-line bg-surface p-3">
          <div class="mb-1 text-xs font-semibold text-muted">Memory</div>
          <UplotChart :time="memChart.time" :series="memChart.series" unit="B" />
        </div>
      </div>

      <!-- grouping controls + aggregate table -->
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="mr-auto text-sm font-semibold text-fg">Breakdown</h2>
        <span class="text-xs text-faint">group by</span>
        <UiSelect :model-value="by" @update:model-value="setBy" :options="BY_OPTS" />
        <input v-if="by === 'label'" :value="labelKey" @change="setLabelKey($event.target.value.trim())"
          placeholder="label key" class="w-36 rounded-lg border border-line bg-surface2 px-2.5 py-1.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        <span v-if="asOf" class="text-xs text-faint">as of {{ asOf }}</span>
      </div>

      <DataTable :columns="aggCols" :rows="agg.groups" :row-key="(r) => r.group ?? '—'" clickable
        :filterable="agg.groups.length > 8" filter-placeholder="Filter groups…"
        :initial-sort="{ key: 'cpu_millicores', dir: 'desc' }"
        empty="No pods reported yet — is the k8s-cluster agent deployed?"
        @row-click="(r) => selectGroup(r.group)">
        <template #cell-group="{ row }">
          <span class="font-medium" :class="sel === row.group ? 'text-accent' : 'text-fg'">{{ row.group ?? '—' }}</span>
        </template>
        <template #cell-cpu_millicores="{ row }">{{ fmtCores(row.cpu_millicores) }}</template>
        <template #cell-mem_bytes="{ row }">{{ fmtBytes(row.mem_bytes) }}</template>
      </DataTable>

      <!-- drill-down: containers of the selected group -->
      <template v-if="sel">
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
