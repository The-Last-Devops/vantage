<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { minLoad } from '../lib/minLoad'

const route = useRoute()
const router = useRouter()

const monitors = ref([])
const events = ref([])
const loading = ref(true)
const err = ref('')
let timer = null

const KINDS = {
  http: 'HTTP(s)', keyword: 'HTTP keyword', tcp: 'TCP port', ping: 'Ping', postgres: 'PostgreSQL',
  mysql: 'MySQL', mongodb: 'MongoDB', redis: 'Redis', rabbitmq: 'RabbitMQ', dns: 'DNS', tls: 'TLS cert', push: 'Push',
}

const downOnly = computed(() => route.query.status === 'down')
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const nsMonitors = computed(() =>
  selectedNs.value.length ? monitors.value.filter((m) => selectedNs.value.includes(m.namespace)) : monitors.value,
)
const shown = computed(() => (downOnly.value ? nsMonitors.value.filter((m) => m.enabled && m.up === false) : nsMonitors.value))
const upPct = (m) => (m.recent && m.recent.length ? Math.round((m.recent.filter(Boolean).length / m.recent.length) * 100) : null)
// How much wall-clock the recent-beats window covers (count × interval).
const windowSecs = (m) => (m.recent?.length || 0) * (m.interval_secs || 60)

const stats = computed(() => {
  let up = 0, down = 0, paused = 0
  for (const m of nsMonitors.value) {
    if (!m.enabled) paused++
    else if (m.up === true) up++
    else if (m.up === false) down++
  }
  return { up, down, paused, total: nsMonitors.value.length }
})

// ---- table ----
const STATE = { up: ['ok', 'Up'], down: ['down', 'Down'], pending: ['warn', 'Pending'], paused: ['muted', 'Paused'] }
const stateKey = (m) => (!m.enabled ? 'paused' : m.up === true ? 'up' : m.up === false ? 'down' : 'pending')
const tableRows = computed(() =>
  shown.value.map((m) => ({
    ...m,
    state: STATE[stateKey(m)][1],
    typeLabel: KINDS[m.kind] || m.kind,
    uptime: upPct(m),
  })),
)
const columns = [
  { key: 'state', label: 'State', sortable: true, width: '112px' },
  { key: 'name', label: 'Name', sortable: true, nowrap: false },
  { key: 'namespace', label: 'Namespace', sortable: true },
  { key: 'typeLabel', label: 'Type', sortable: true },
  { key: 'target', label: 'Target', mono: true, nowrap: false },
  { key: 'uptime', label: 'Uptime', sortable: true, align: 'right', width: '90px' },
  { key: 'history', label: 'History', width: '160px' },
  { key: 'actions', label: '', align: 'right', width: '88px' },
]
const selectedIds = ref([])

const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const openCreate = () => router.push({ name: 'monitor-new', query: nsq.value })
const openEdit = (m) => router.push({ name: 'monitor-edit', params: { id: m.id }, query: nsq.value })
const openDetail = (m) => router.push({ name: 'monitor', params: { id: m.id }, query: nsq.value })

async function load() {
  const first = loading.value
  try { const w = api.get('/api/monitors'); monitors.value = await (first ? minLoad(w) : w); err.value = '' }
  catch { if (!monitors.value.length) err.value = 'Failed to load monitors' }
  try { events.value = await api.get('/api/events?range=7d') } catch { events.value = [] }
  loading.value = false
}
async function removeMonitor(m) {
  if (!confirm(`Delete service "${m.name}"?`)) return
  try { await api.del(`/api/monitors/${m.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}
async function bulkDelete(rows) {
  if (!confirm(`Delete ${rows.length} service(s)? This cannot be undone.`)) return
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
    <template #title-after><span class="text-sm text-faint">{{ stats.total }} services<span v-if="stats.down" class="text-rose-500"> · {{ stats.down }} down</span></span></template>
    <template #actions>
      <button @click="openCreate" class="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-accentfg hover:opacity-90">
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> Add service
      </button>
    </template>
    <PageLoader v-if="loading" />
    <div v-else class="space-y-4">
      <p v-if="err" class="rounded-lg border border-rose-500/30 bg-rose-500/5 px-3 py-2 text-sm text-rose-500">{{ err }}</p>

      <!-- quick stats -->
      <div v-if="!downOnly" class="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface px-4 py-3"><div class="text-[11px] uppercase tracking-wider text-faint">Up</div><div class="text-2xl font-semibold tabular-nums text-accent">{{ stats.up }}</div></div>
        <div class="rounded-xl border border-line bg-surface px-4 py-3"><div class="text-[11px] uppercase tracking-wider text-faint">Down</div><div class="text-2xl font-semibold tabular-nums" :class="stats.down ? 'text-rose-500' : 'text-muted'">{{ stats.down }}</div></div>
        <div class="rounded-xl border border-line bg-surface px-4 py-3"><div class="text-[11px] uppercase tracking-wider text-faint">Paused</div><div class="text-2xl font-semibold tabular-nums text-faint">{{ stats.paused }}</div></div>
        <div class="rounded-xl border border-line bg-surface px-4 py-3"><div class="text-[11px] uppercase tracking-wider text-faint">Total</div><div class="text-2xl font-semibold tabular-nums text-fg">{{ stats.total }}</div></div>
      </div>

      <DataTable v-model:selected="selectedIds" :columns="columns" :rows="tableRows" :row-key="(r) => r.id"
        selectable clickable @row-click="openDetail" :filter-keys="['name', 'namespace', 'typeLabel', 'target']"
        filter-placeholder="Filter services…" :empty="downOnly ? 'Nothing down. 🎉' : 'No services yet.'">
        <template #bulk="{ selected, disabled }">
          <button :disabled="disabled" @click="bulkDelete(selected)" class="rounded-lg border border-rose-500/35 px-2.5 py-1.5 text-xs font-medium text-rose-500 hover:bg-rose-500/10 disabled:cursor-not-allowed disabled:opacity-40">Delete</button>
        </template>
        <template #cell-state="{ row }"><StatePill :tone="STATE[stateKey(row)][0]" :label="row.state" /></template>
        <template #cell-name="{ row }"><span class="font-medium text-fg">{{ row.name }}</span></template>
        <template #cell-namespace="{ row }"><span class="text-muted">{{ row.namespace }}</span></template>
        <template #cell-typeLabel="{ row }"><span class="text-muted">{{ row.typeLabel }}</span></template>
        <template #cell-target="{ row }"><span class="text-faint">{{ row.target || '—' }}</span></template>
        <template #cell-uptime="{ row }"><span :class="row.uptime == null ? 'text-faint' : row.uptime >= 99 ? 'text-accent' : row.uptime >= 90 ? 'text-amber-500' : 'text-rose-500'" v-tip="row.recent?.length ? `${row.uptime}% over the last ${row.recent.length} checks (~${fmtDur(windowSecs(row))})` : 'No checks yet'">{{ row.uptime == null ? 'N/A' : row.uptime + '%' }}</span></template>
        <template #cell-history="{ row }">
          <span class="flex items-center gap-2" v-tip="row.recent?.length ? `Last ${row.recent.length} checks · spans ~${fmtDur(windowSecs(row))}` : 'No checks yet'">
            <span class="flex items-end gap-px">
              <span v-for="(u, i) in (row.recent || []).slice(-24)" :key="i" class="h-3.5 w-1 rounded-sm" :class="u ? 'bg-accent' : 'bg-rose-500'"></span>
            </span>
            <span v-if="row.recent?.length" class="text-[11px] tabular-nums text-faint">{{ fmtDur(windowSecs(row)) }}</span>
            <span v-else class="text-[11px] text-faint">no checks</span>
          </span>
        </template>
        <template #cell-actions="{ row }">
          <div class="flex items-center justify-end gap-1">
            <button @click.stop="openEdit(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="`Edit`"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg></button>
            <button @click.stop="removeMonitor(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-rose-500" v-tip="`Delete`"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg></button>
          </div>
        </template>
      </DataTable>

      <!-- recent events feed -->
      <section v-if="!downOnly" class="space-y-2">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-faint">Recent events</h2>
        <p v-if="!shownEvents.length" class="rounded-xl border border-line bg-surface p-5 text-center text-xs text-muted">No status changes recorded recently.</p>
        <div v-else class="overflow-x-auto rounded-xl border border-line bg-surface">
          <table class="w-full text-xs">
            <thead><tr class="border-b border-line-strong bg-headbg text-left text-[10px] uppercase tracking-wider text-muted">
              <th class="px-3 py-1.5 font-semibold">Status</th>
              <th class="px-3 py-1.5 font-semibold">Service</th>
              <th class="px-3 py-1.5 font-semibold">When</th>
              <th class="px-3 py-1.5 font-semibold">Duration</th>
              <th class="px-3 py-1.5 font-semibold">Message</th>
            </tr></thead>
            <tbody>
              <tr v-for="(e, i) in shownEvents" :key="i" class="border-b border-line last:border-0 hover:bg-hover">
                <td class="px-3 py-1.5"><StatePill :tone="e.up ? 'ok' : 'down'" :label="e.up ? 'Up' : 'Down'" /></td>
                <td class="px-3 py-1.5"><RouterLink :to="{ name: 'monitor', params: { id: e.monitor_id } }" class="text-accent hover:underline">{{ e.name }}</RouterLink></td>
                <td class="whitespace-nowrap px-3 py-1.5 tabular-nums text-muted">{{ evTime(e.at) }}</td>
                <td class="whitespace-nowrap px-3 py-1.5 tabular-nums text-muted">{{ fmtDur(stateDur(i).secs) }}<span v-if="stateDur(i).ongoing" class="text-faint"> · ongoing</span></td>
                <td class="px-3 py-1.5 text-muted">{{ evMessage(e) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </AppShell>
</template>
