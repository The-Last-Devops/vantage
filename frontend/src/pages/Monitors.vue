<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import UtilBar from '../components/UtilBar.vue'
import EventStream from '../components/EventStream.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { useCached } from '../lib/cache'

const route = useRoute()
const router = useRouter()

const monitors = ref([])
const events = ref([])
const err = ref('')
let timer = null

const KINDS = {
  http: 'HTTP(s)', keyword: 'HTTP keyword', tcp: 'TCP port', ping: 'Ping', postgres: 'PostgreSQL',
  mysql: 'MySQL', mongodb: 'MongoDB', redis: 'Redis', rabbitmq: 'RabbitMQ', dns: 'DNS', tls: 'TLS cert', push: 'Push',
}
// DS glyph per check kind (falls back to the generic service icon)
const KIND_ICON = {
  http: 'globe', keyword: 'globe', tls: 'shield', dns: 'globe', ping: 'wifi-off',
  tcp: 'network', postgres: 'service', mysql: 'service', mongodb: 'service', redis: 'service',
  rabbitmq: 'service', push: 'pulse',
}
const kindIcon = (k) => KIND_ICON[k] || 'service'

const downOnly = computed(() => route.query.status === 'down')
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const nsMonitors = computed(() =>
  selectedNs.value.length ? monitors.value.filter((m) => selectedNs.value.includes(m.namespace)) : monitors.value,
)
const shown = computed(() => (downOnly.value ? nsMonitors.value.filter((m) => m.enabled && m.up === false) : nsMonitors.value))
// Uptime % over the last 24h, computed server-side (null if no beats in window).
const upPct = (m) => (m.uptime_24h == null ? null : Math.round(m.uptime_24h))
// Whether the 24h trend has at least one real beat (else show "no checks").
const hasTrend = (m) => (m.trend_24h || []).some((u) => u != null)

const stats = computed(() => {
  let up = 0, down = 0, paused = 0, warn = 0
  const ups = []
  for (const m of nsMonitors.value) {
    if (!m.enabled) paused++
    else if (m.up === true) up++
    else if (m.up === false) down++
    else warn++
    const u = upPct(m)
    if (u != null) ups.push(u)
  }
  const active = up + down + warn
  const avg = ups.length ? Math.round(ups.reduce((a, b) => a + b, 0) / ups.length) : null
  return { up, down, warn, paused, active, total: nsMonitors.value.length, avg }
})

// ---- table ----
const STATE = { up: ['ok', 'Up'], down: ['down', 'Down'], pending: ['warn', 'Pending'], paused: ['muted', 'Paused'] }
const stateKey = (m) => (!m.enabled ? 'paused' : m.up === true ? 'up' : m.up === false ? 'down' : 'pending')
// row-tone for the ops table wash: down → red, pending → warn (amber), else none
const rowTone = (m) => (!m.enabled ? null : m.up === false ? 'down' : m.up == null ? 'warn' : null)
const tableRows = computed(() =>
  shown.value.map((m) => ({
    ...m,
    stateKey: stateKey(m),
    state: STATE[stateKey(m)][1],
    typeLabel: KINDS[m.kind] || m.kind,
    uptime: upPct(m),
  })),
)
// search box lives in the toolbar (like Infrastructure); filter rows client-side
const q = ref('')
const filteredRows = computed(() => {
  const s = q.value.trim().toLowerCase()
  if (!s) return tableRows.value
  return tableRows.value.filter((r) =>
    [r.name, r.namespace, r.target, r.typeLabel].some((v) => (v || '').toLowerCase().includes(s)),
  )
})
const columns = [
  { key: 'name', label: 'Service', sortable: true, nowrap: false },
  { key: 'uptime', label: 'Uptime 24h', sortable: true, width: '128px' },
  { key: 'latency_ms', label: 'Latency', sortable: true, align: 'right', width: '120px' },
  { key: 'history', label: 'Trend', width: '160px' },
]
const selectedIds = ref([])

const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const openCreate = () => router.push({ name: 'monitor-new', query: nsq.value })
const openEdit = (m) => router.push({ name: 'monitor-edit', params: { id: m.id }, query: nsq.value })
const openDetail = (m) => router.push({ name: 'monitor', params: { id: m.id }, query: nsq.value })

const { loaded, reload: load } = useCached({
  key: () => 'monitors',
  load: async () => {
    let mons = monitors.value
    let evs = []
    let error = ''
    try { mons = await api.get('/api/monitors'); error = '' }
    catch { if (!monitors.value.length) error = 'Failed to load monitors'; mons = monitors.value }
    try { evs = await api.get('/api/events?range=7d') } catch { evs = [] }
    return { monitors: mons, events: evs, err: error }
  },
  apply: (d) => { monitors.value = d.monitors; events.value = d.events; err.value = d.err },
})
async function removeMonitor(m) {
  if (!(await confirm({ title: 'Delete service?', message: `"${m.name}" and its check history are removed permanently. This cannot be undone.`, danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/monitors/${m.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}
async function bulkDelete(rows) {
  if (!(await confirm({ title: `Delete ${rows.length} service(s)?`, message: 'They are removed along with their collected metrics. This cannot be undone.', danger: true, confirmText: `Delete ${rows.length}` }))) return
  await Promise.all(rows.map((m) => api.del(`/api/monitors/${m.id}`).catch(() => {})))
  selectedIds.value = []
  await load()
}

// ---- recent-events feed ----
const shownEvents = computed(() => {
  if (!selectedNs.value.length) return events.value
  const ids = new Set(nsMonitors.value.map((m) => m.id))
  return events.value.filter((e) => ids.has(e.monitor_id))
})
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
const evMessage = (e) => e.message || (e.up ? 'Recovered' : 'Down')
const fmtDur = (s) => {
  s = Math.max(0, Math.round(s))
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  if (m) return `${m}m ${sec}s`
  return `${sec}s`
}
const stateDur = (i) => {
  const list = shownEvents.value
  const start = new Date(list[i].at).getTime()
  for (let j = i - 1; j >= 0; j--) {
    if (list[j].monitor_id === list[i].monitor_id)
      return { secs: (new Date(list[j].at).getTime() - start) / 1000, ongoing: false }
  }
  return { secs: (Date.now() - start) / 1000, ongoing: true }
}

onMounted(async () => { await load(); timer = setInterval(load, 10000) })
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell :title="downOnly ? 'Services — Down' : 'Services'">
    <template #title-after><span class="text-sm text-faint">{{ stats.total }} services<span v-if="stats.down" class="text-down"> · {{ stats.down }} down</span></span></template>
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <p v-if="err" class="rounded-lg border border-down/30 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>

      <!-- KPI strip -->
      <div v-if="!downOnly" class="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface px-4 py-3">
          <div class="text-micro uppercase tracking-wider text-faint">Services up</div>
          <div class="mt-1 font-mono text-metric text-ok">{{ stats.up }} <span class="text-base font-normal text-faint">/ {{ stats.active }} up</span></div>
        </div>
        <div class="rounded-xl border px-4 py-3" :class="stats.down ? 'border-down/30 bg-down/12' : 'border-line bg-surface'">
          <div class="text-micro uppercase tracking-wider text-faint">Down</div>
          <div class="mt-1 font-mono text-metric" :class="stats.down ? 'text-down' : 'text-muted'">{{ stats.down }}</div>
        </div>
        <div class="rounded-xl border px-4 py-3" :class="stats.warn ? 'border-warn/30 bg-warn/12' : 'border-line bg-surface'">
          <div class="text-micro uppercase tracking-wider text-faint">Pending</div>
          <div class="mt-1 font-mono text-metric" :class="stats.warn ? 'text-warn' : 'text-muted'">{{ stats.warn }}</div>
        </div>
        <div class="rounded-xl border border-line bg-surface px-4 py-3">
          <div class="text-micro uppercase tracking-wider text-faint">Avg uptime</div>
          <div class="mt-1 font-mono text-metric" :class="stats.avg == null ? 'text-faint' : stats.avg >= 99 ? 'text-ok' : stats.avg >= 90 ? 'text-warn' : 'text-down'">{{ stats.avg == null ? '—' : stats.avg + '%' }}</div>
          <div class="mt-0.5 text-micro text-faint">over the last 24h</div>
        </div>
      </div>

      <!-- toolbar: search + add sit together on the left (mirrors Infrastructure) -->
      <div class="flex flex-wrap items-center gap-3">
        <div class="relative">
          <VIcon name="search" :size="15" class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <input v-model="q" type="search" placeholder="Search services…"
            class="w-72 rounded-lg border border-line bg-surface2 py-2 pl-8 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none sm:w-96" />
        </div>
        <button @click="openCreate" class="flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <VIcon name="plus" :size="16" /> Add service
        </button>
      </div>

      <!-- table + events side panel -->
      <div class="grid grid-cols-1 gap-5 xl:grid-cols-[1fr_340px]">
        <DataTable v-model:selected="selectedIds" :columns="columns" :rows="filteredRows" :row-key="(r) => r.id"
          :row-tone="rowTone" selectable clickable @row-click="openDetail" :filterable="false"
          :empty="downOnly ? 'Nothing down. 🎉' : 'No services yet.'">
          <template #bulk="{ selected, disabled }">
            <button :disabled="disabled" @click="bulkDelete(selected)" class="rounded-lg border border-down/35 px-2.5 py-1.5 text-xs font-medium text-down hover:bg-down/10 disabled:cursor-not-allowed disabled:opacity-40">Delete</button>
          </template>
          <template #cell-name="{ row }">
            <div class="flex items-center gap-3">
              <span class="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-line bg-surface2"
                :class="row.stateKey === 'down' ? 'text-down' : row.stateKey === 'pending' ? 'text-warn' : row.stateKey === 'paused' ? 'text-faint' : 'text-accent'">
                <VIcon :name="kindIcon(row.kind)" :size="18" />
              </span>
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <StatePill :tone="STATE[row.stateKey][0]" :label="row.state" />
                  <span class="truncate font-mono text-sm text-fg">{{ row.name }}</span>
                </div>
                <div class="mt-0.5 truncate text-micro text-faint">{{ row.namespace }} · {{ row.typeLabel }}<span v-if="row.target"> · {{ row.target }}</span></div>
              </div>
            </div>
          </template>
          <template #cell-uptime="{ row }">
            <span class="font-mono text-sm tabular-nums" :class="row.uptime == null ? 'text-faint' : row.uptime >= 99 ? 'text-ok' : row.uptime >= 90 ? 'text-warn' : 'text-down'"
              v-tip="row.uptime == null ? 'No checks in the last 24h' : `${row.uptime}% uptime over the last 24 hours`">{{ row.uptime == null ? 'N/A' : row.uptime + '%' }}</span>
          </template>
          <template #cell-latency_ms="{ row }">
            <UtilBar v-if="row.latency_ms != null" :value="row.latency_ms" :max="1000" width="w-12"><span class="font-mono text-sm tabular-nums text-fg">{{ row.latency_ms }} ms</span></UtilBar>
            <span v-else class="font-mono text-sm text-faint">—</span>
          </template>
          <template #cell-history="{ row }">
            <span class="flex items-center gap-2" v-tip="'Last 24 hours · 1 bar = 1 hour'">
              <span class="flex items-end gap-px">
                <span v-for="(u, i) in (row.trend_24h || [])" :key="i" class="h-3.5 w-1 rounded-sm"
                  :class="u == null ? 'bg-line' : u ? 'bg-ok' : 'bg-down'"
                  v-tip="`${23 - i}h ago: ${u == null ? 'no data' : u ? 'up' : 'down'}`"></span>
              </span>
              <span v-if="!hasTrend(row)" class="text-micro text-faint">no checks</span>
            </span>
          </template>
          <template #row-actions="{ row }">
            <button @click.stop="openEdit(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="`Edit`"><VIcon name="settings" :size="16" /></button>
            <button @click.stop="removeMonitor(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-down" v-tip="`Delete`"><VIcon name="trash" :size="16" /></button>
          </template>
        </DataTable>

        <!-- recent events side panel -->
        <EventStream v-if="!downOnly" :events="shownEvents" :ev-time="evTime" :ev-message="evMessage" :state-dur="stateDur" :fmt-dur="fmtDur" class="h-fit" />
      </div>
    </div>
  </AppShell>
</template>
