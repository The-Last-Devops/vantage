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
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const wsMonitors = computed(() =>
  selectedWs.value.length ? monitors.value.filter((m) => selectedWs.value.includes(m.workspace)) : monitors.value,
)
const shown = computed(() => (downOnly.value ? wsMonitors.value.filter((m) => m.enabled && m.up === false) : wsMonitors.value))
// Uptime % over the last 24h, computed server-side (null if no beats in window).
const upPct = (m) => (m.uptime_24h == null ? null : Math.round(m.uptime_24h))
// Whether the 24h trend has at least one real beat (else show "no checks").
const hasTrend = (m) => (m.trend_24h || []).some((u) => u != null)

const stats = computed(() => {
  let up = 0, down = 0, paused = 0, warn = 0
  const ups = []
  for (const m of wsMonitors.value) {
    if (!m.enabled) paused++
    else if (m.up === true) up++
    else if (m.up === false) down++
    else warn++
    const u = upPct(m)
    if (u != null) ups.push(u)
  }
  const active = up + down + warn
  const avg = ups.length ? Math.round(ups.reduce((a, b) => a + b, 0) / ups.length) : null
  return { up, down, warn, paused, active, total: wsMonitors.value.length, avg }
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
    [r.name, r.workspace, r.target, r.typeLabel].some((v) => (v || '').toLowerCase().includes(s)),
  )
})
const columns = [
  { key: 'name', label: 'Service', sortable: true, nowrap: false },
  { key: 'uptime', label: 'Uptime 24h', sortable: true, width: '128px' },
  { key: 'latency_ms', label: 'Latency', sortable: true, align: 'right', width: '120px' },
  { key: 'history', label: 'Trend', width: '160px' },
]
const selectedIds = ref([])

const nsq = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
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
    try { evs = await api.get('/api/events?range=30d') } catch { evs = [] }
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
  if (!selectedWs.value.length) return events.value
  const ids = new Set(wsMonitors.value.map((m) => m.id))
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

// ---- Down view (downOnly=true): who's down now + resolved incidents (30d) ----
const fmtAgo = (s) => (s == null ? '—' : fmtDur(s) + ' ago')
const agoSecs = (iso) => (Date.now() - new Date(iso).getTime()) / 1000
const WINDOW_30D = 30 * 86400

const monById = computed(() => Object.fromEntries(monitors.value.map((m) => [m.id, m])))
const downNow = computed(() => wsMonitors.value.filter((m) => m.enabled && m.up === false))

// Pair status transitions (newest-first) into resolved incidents; leftover unresolved
// downs are the ongoing outages (their start time is used as "down since").
const incident30 = computed(() => {
  const pendingUp = {} // monitor_id -> recovery timestamp (ms) awaiting its down
  const history = []
  const downSince = {} // monitor_id -> ISO start of the ongoing outage
  for (const e of shownEvents.value) {
    const mid = e.monitor_id
    const t = new Date(e.at).getTime()
    if (e.up) {
      pendingUp[mid] = t
    } else if (pendingUp[mid] != null) {
      const m = monById.value[mid] || {}
      history.push({
        id: `${mid}-${e.at}`, monitor_id: mid, name: e.name, kind: m.kind, workspace: m.workspace,
        started_at: e.at, duration_s: Math.max(0, (pendingUp[mid] - t) / 1000), cause: e.message || 'Down', resolved: true,
      })
      delete pendingUp[mid]
    } else if (downSince[mid] == null) {
      downSince[mid] = e.at
    }
  }
  return { history, downSince }
})
const history = computed(() => incident30.value.history)
const downSince = (mid) => incident30.value.downSince[mid]

const matchQ = (r, ...fields) => {
  const s = q.value.trim().toLowerCase()
  return !s || fields.some((v) => (v || '').toLowerCase().includes(s))
}
const downNowFiltered = computed(() => downNow.value.filter((m) => matchQ(m, m.name, m.workspace, m.target, m.message)))
const historyFiltered = computed(() => history.value.filter((h) => matchQ(h, h.name, h.workspace, h.cause)))

const stats30 = computed(() => {
  const h = history.value
  const totalDur = h.reduce((a, i) => a + i.duration_s, 0)
  const now = Date.now()
  const ongoing = downNow.value.reduce((a, m) => a + (downSince(m.id) ? agoSecs(downSince(m.id)) : 0), 0)
  const uptime_pct = Math.max(0, Math.min(100, (1 - (totalDur + ongoing) / WINDOW_30D) * 100))
  const mttr_s = h.length ? totalDur / h.length : null
  let streak_s = null
  if (downNow.value.length) streak_s = 0
  else {
    const lastResolved = h.reduce((mx, i) => Math.max(mx, new Date(i.started_at).getTime() + i.duration_s * 1000), 0)
    streak_s = lastResolved ? (now - lastResolved) / 1000 : null
  }
  return { uptime_pct, incidents: h.length, mttr_s, streak_s }
})

// Standard table header (matches DataTable.vue).
const TH = 'border-b border-line2 bg-head px-4 py-3 text-xs font-extrabold uppercase tracking-wide text-fg'

onMounted(async () => { await load(); timer = setInterval(load, 10000) })
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell :title="downOnly ? 'Services — Down' : 'Services'">
    <template #title-after><span class="text-sm text-faint">{{ stats.total }} services<span v-if="stats.down" class="text-down"> · {{ stats.down }} down</span></span></template>
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <p v-if="err" class="rounded-lg border border-down/30 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>

      <!-- summary strip: one thin row of 4 figures; a zero figure is dimmed -->
      <div v-if="!downOnly" class="grid grid-cols-2 overflow-hidden rounded-xl border border-line bg-surface sm:grid-cols-4 sm:divide-x sm:divide-line">
        <div class="px-4 py-2.5">
          <div class="text-micro uppercase tracking-wider text-faint">Services up</div>
          <div class="mt-0.5 font-mono text-h1 text-ok">{{ stats.up }}<span class="text-sm font-normal text-faint"> / {{ stats.active }}</span></div>
        </div>
        <div class="px-4 py-2.5">
          <div class="text-micro uppercase tracking-wider text-faint">Down</div>
          <div class="mt-0.5 font-mono text-h1" :class="stats.down ? 'text-down' : 'text-cap'">{{ stats.down }}</div>
        </div>
        <div class="px-4 py-2.5">
          <div class="text-micro uppercase tracking-wider text-faint">Pending</div>
          <div class="mt-0.5 font-mono text-h1" :class="stats.warn ? 'text-warn' : 'text-cap'">{{ stats.warn }}</div>
        </div>
        <div class="px-4 py-2.5">
          <div class="text-micro uppercase tracking-wider text-faint">Avg uptime 24h</div>
          <div class="mt-0.5 font-mono text-h1" :class="stats.avg == null ? 'text-cap' : stats.avg >= 99 ? 'text-ok' : stats.avg >= 90 ? 'text-warn' : 'text-down'">{{ stats.avg == null ? '—' : stats.avg + '%' }}</div>
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

      <!-- normal Services view: table + recent-events rail (stacked; rail when wide) -->
      <div v-if="!downOnly" class="grid grid-cols-1 gap-5 min-[1080px]:grid-cols-[1fr_330px]">
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
                <div class="mt-0.5 truncate text-micro text-faint">{{ row.workspace }} · {{ row.typeLabel }}<span v-if="row.target"> · {{ row.target }}</span></div>
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
        <EventStream :events="shownEvents" :ev-time="evTime" :ev-message="evMessage" :state-dur="stateDur" :fmt-dur="fmtDur" class="min-[1080px]:sticky min-[1080px]:top-6" />
      </div>

      <!-- Down view: all-clear banner + stats, or a "Down now" table; plus recent downtime -->
      <template v-else>
        <!-- ALL CLEAR -->
        <template v-if="!downNow.length">
          <section class="rounded-xl border border-ok/30 bg-gradient-to-br from-ok/10 to-transparent p-5">
            <div class="flex flex-wrap items-center justify-between gap-4">
              <div class="flex items-center gap-4">
                <span class="grid h-12 w-12 shrink-0 place-items-center rounded-full bg-ok/15 text-ok"><VIcon name="check-circle" :size="26" /></span>
                <div>
                  <div class="text-h2 font-semibold text-fg">All services are up</div>
                  <div class="mt-0.5 text-sm text-muted">{{ stats.total }} / {{ stats.total }} responding · 0 down · 0 degraded</div>
                </div>
              </div>
              <div v-if="stats30.streak_s != null" class="text-right text-sm text-muted">Last incident<div class="font-mono text-fg">{{ fmtAgo(stats30.streak_s) }}</div></div>
            </div>
          </section>
          <div class="grid grid-cols-2 overflow-hidden rounded-xl border border-line bg-surface sm:grid-cols-4 sm:divide-x sm:divide-line">
            <div class="px-4 py-2.5"><div class="text-micro uppercase tracking-wider text-faint">Uptime 30d</div><div class="mt-0.5 font-mono text-h1 text-ok">{{ stats30.uptime_pct.toFixed(2) }}%</div></div>
            <div class="px-4 py-2.5"><div class="text-micro uppercase tracking-wider text-faint">Incidents 30d</div><div class="mt-0.5 font-mono text-h1" :class="stats30.incidents ? 'text-fg' : 'text-cap'">{{ stats30.incidents }}</div></div>
            <div class="px-4 py-2.5"><div class="text-micro uppercase tracking-wider text-faint">MTTR 30d</div><div class="mt-0.5 font-mono text-h1" :class="stats30.mttr_s == null ? 'text-cap' : 'text-fg'">{{ stats30.mttr_s == null ? '—' : fmtDur(stats30.mttr_s) }}</div></div>
            <div class="px-4 py-2.5"><div class="text-micro uppercase tracking-wider text-faint">Current streak</div><div class="mt-0.5 font-mono text-h1 text-fg">{{ stats30.streak_s == null ? '—' : fmtDur(stats30.streak_s) }}</div></div>
          </div>
        </template>

        <!-- DOWN NOW -->
        <section v-else class="space-y-3">
          <div class="flex items-center gap-2">
            <VIcon name="wifi-off" :size="18" class="text-down" />
            <h2 class="text-h2 font-semibold text-fg">Down now</h2>
            <span class="rounded-pill bg-down/15 px-2 py-0.5 text-micro font-semibold text-down">{{ downNowFiltered.length }}</span>
          </div>
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead><tr class="text-left"><th :class="TH">Service</th><th :class="TH">Down since</th><th :class="TH" class="text-right">Uptime 24h</th><th :class="TH">Cause</th><th :class="TH"></th></tr></thead>
                <tbody>
                  <tr v-for="m in downNowFiltered" :key="m.id" @click="openDetail(m)" class="cursor-pointer border-b border-l-2 border-line/60 border-l-down bg-down/5 last:border-b-0 hover:bg-down/10">
                    <td class="px-4 py-2.5">
                      <div class="flex items-center gap-3">
                        <span class="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-line bg-surface2 text-down"><VIcon :name="kindIcon(m.kind)" :size="18" /></span>
                        <div class="min-w-0">
                          <div class="flex items-center gap-2"><StatePill tone="down" label="Down" /><span class="truncate font-mono text-sm text-fg">{{ m.name }}</span></div>
                          <div class="mt-0.5 truncate text-micro text-faint">{{ m.workspace }} · {{ KINDS[m.kind] || m.kind }}<span v-if="m.target"> · {{ m.target }}</span></div>
                        </div>
                      </div>
                    </td>
                    <td class="px-4 py-2.5 font-mono tabular-nums text-down">{{ downSince(m.id) ? fmtDur(agoSecs(downSince(m.id))) : '—' }}</td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums text-down">{{ upPct(m) == null ? 'N/A' : upPct(m) + '%' }}</td>
                    <td class="px-4 py-2.5 text-muted">{{ m.message || '—' }}</td>
                    <td class="px-4 py-2.5 text-right">
                      <button @click.stop="openEdit(m)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="'Edit'"><VIcon name="settings" :size="16" /></button>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </section>

        <!-- RECENT DOWNTIME (always, when there's history) -->
        <section v-if="historyFiltered.length" class="space-y-3">
          <div class="flex items-center gap-2">
            <VIcon name="logs" :size="18" class="text-faint" />
            <h2 class="text-h2 font-semibold text-fg">Recent downtime</h2>
            <span class="rounded-pill bg-surface2 px-2 py-0.5 text-micro text-muted">{{ historyFiltered.length }}</span>
            <span class="text-xs text-faint">last 30 days</span>
          </div>
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead><tr class="text-left"><th :class="TH">Service</th><th :class="TH">Started</th><th :class="TH" class="text-right">Duration</th><th :class="TH">Cause</th><th :class="TH" class="text-right">Status</th></tr></thead>
                <tbody>
                  <tr v-for="h in historyFiltered" :key="h.id" class="border-b border-line/60 last:border-b-0">
                    <td class="px-4 py-2.5"><div class="font-mono text-fg">{{ h.name }}</div><div class="mt-0.5 text-micro text-faint">{{ h.workspace }} · {{ KINDS[h.kind] || h.kind }}</div></td>
                    <td class="px-4 py-2.5 font-mono tabular-nums text-muted">{{ evTime(h.started_at) }}</td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums text-fg">{{ fmtDur(h.duration_s) }}</td>
                    <td class="px-4 py-2.5 text-muted">{{ h.cause }}</td>
                    <td class="px-4 py-2.5 text-right"><StatePill tone="ok" label="Resolved" /></td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </section>
      </template>
    </div>
  </AppShell>
</template>
