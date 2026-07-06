<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { minLoad } from '../lib/minLoad'
import UplotChart from '../components/UplotChart.vue'
import MonitorHeartbeatBar from '../components/MonitorHeartbeatBar.vue'
import MonitorIncidentsList from '../components/MonitorIncidentsList.vue'
import MonitorDebugCard from '../components/MonitorDebugCard.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'

const route = useRoute()
const router = useRouter()
const id = route.params.id

const KINDS = {
  http: 'HTTP(s)', keyword: 'HTTP keyword', tcp: 'TCP port', ping: 'Ping', postgres: 'PostgreSQL',
  mysql: 'MySQL', mongodb: 'MongoDB', redis: 'Redis', rabbitmq: 'RabbitMQ', dns: 'DNS', tls: 'TLS cert', push: 'Push',
}
const KIND_ICON = {
  http: 'globe', keyword: 'globe', tls: 'shield', dns: 'globe', ping: 'wifi-off',
  tcp: 'network', postgres: 'service', mysql: 'service', mongodb: 'service', redis: 'service',
  rabbitmq: 'service', push: 'pulse',
}
const kindIcon = (k) => KIND_ICON[k] || 'service'

const m = ref(null)
const hb = ref({ t: [], latency: [], up: [] })
const events = ref([])
const debug = ref(null)
const rules = ref([]) // alert rules covering this service (own + workspace-wide)
const range = ref('24h')
const nsq = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
async function loadRules() {
  try { rules.value = await api.get(`/api/monitors/${id}/alerts`) } catch { rules.value = [] }
}
const err = ref('')
let timer = null

const RANGES = [
  { v: '1h', label: '1h' },
  { v: '6h', label: '6h' },
  { v: '24h', label: '24h' },
  { v: '7d', label: '7d' },
  { v: '30d', label: '30d' },
  { v: '90d', label: '90d' },
  { v: '1y', label: '1y' },
]
const SPAN = { '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800, '30d': 2592000, '90d': 7776000, '1y': 31536000 }
const spanSeconds = computed(() => SPAN[range.value] || 86400)

// Pair status transitions (newest-first from the API) into down incidents:
// a down event opens an incident, the next transition (up) closes it.
const incidents = computed(() => {
  const asc = [...events.value].reverse() // oldest→newest
  const out = []
  for (let i = 0; i < asc.length; i++) {
    if (asc[i].up) continue
    const start = new Date(asc[i].at).getTime()
    const next = asc[i + 1]
    const end = next ? new Date(next.at).getTime() : null // null = ongoing
    out.push({ at: asc[i].at, reason: asc[i].message || 'Down', start, end, ongoing: !next })
  }
  return out.reverse() // newest first
})
function durTxt(ms) {
  let s = Math.max(0, Math.round(ms / 1000))
  const d = Math.floor(s / 86400); s -= d * 86400
  const h = Math.floor(s / 3600); s -= h * 3600
  const mi = Math.floor(s / 60); s -= mi * 60
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${mi}m`
  if (mi) return `${mi}m ${s}s`
  return `${s}s`
}
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })

const status = computed(() => {
  if (!m.value) return 'pending'
  if (!m.value.enabled) return 'paused'
  if (m.value.up === true) return 'up'
  if (m.value.up === false) return 'down'
  return 'pending'
})
const statusLabel = { up: 'Up', down: 'Down', paused: 'Paused', pending: 'Pending' }
const statusTone = { up: 'ok', down: 'down', paused: 'muted', pending: 'pending' }

const nsq2 = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
function openEdit() { router.push({ name: 'monitor-edit', params: { id }, query: nsq2.value }) }
async function removeMonitor() {
  if (!m.value) return
  if (!(await confirm({ title: 'Delete service?', message: `"${m.value.name}" and its check history are removed permanently. This cannot be undone.`, danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/monitors/${id}`); router.push({ name: 'monitors', query: nsq2.value }) } catch (e) { alert(`Failed (${e.status}).`) }
}

function dur(iso) {
  if (!iso) return '—'
  let s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000)
  const d = Math.floor(s / 86400); s -= d * 86400
  const h = Math.floor(s / 3600); s -= h * 3600
  const mi = Math.floor(s / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${mi}m`
  if (mi) return `${mi}m`
  return `${Math.floor(s)}s`
}
const pct = (v) => (v == null ? '—' : `${v >= 99.95 ? 100 : v.toFixed(2)}%`)
const pushUrl = computed(() => `${location.origin}/pub/push/${m.value?.config?.push_token || ''}`)

const latencySeries = computed(() => [{ name: 'Latency', color: '#2dd4bf', data: hb.value.latency }])

async function loadMeta() {
  try { m.value = await api.get(`/api/monitors/${id}`) } catch (e) { err.value = e.status === 404 ? 'Not found.' : `Failed (${e.status}).` }
}
async function loadHb() {
  try { hb.value = await api.get(`/api/monitors/${id}/heartbeats?range=${range.value}`) } catch { hb.value = { t: [], latency: [], up: [] } }
}
async function loadEvents() {
  try { events.value = await api.get(`/api/monitors/${id}/events?range=${range.value}`) } catch { events.value = [] }
}
async function loadDebug() {
  try { debug.value = await api.get(`/api/monitors/${id}/debug`) } catch { debug.value = null }
}
watch(range, () => { loadHb(); loadEvents() })

const fmtDebug = (d) => (typeof d === 'string' ? d : JSON.stringify(d, null, 2))
function copy(d, e) {
  navigator.clipboard?.writeText(fmtDebug(d))
  const b = e.target; const t = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = t), 1200)
}

onMounted(async () => {
  await minLoad(Promise.all([loadMeta(), loadHb(), loadEvents(), loadDebug(), loadRules()]))
  timer = setInterval(() => { loadMeta(); loadHb(); loadEvents() }, 30000)
})
onUnmounted(() => timer && clearInterval(timer))
</script>

<template>
  <AppShell :title="m?.name || 'Service'" :breadcrumb="[{ label: 'Services', to: { name: 'monitors', query: route.query.ws ? { ws: route.query.ws } : {} } }, { label: m?.name || 'Service' }]">

    <div v-if="err" class="rounded-xl border border-line bg-surface p-6 text-center text-down">{{ err }}</div>
    <PageLoader v-else-if="!m" />
    <div v-else class="space-y-5">
      <!-- hero header -->
      <section class="rounded-xl border border-line bg-surface p-4 sm:p-5">
        <div class="flex flex-col gap-4 sm:flex-row sm:items-start">
          <span class="grid h-12 w-12 shrink-0 place-items-center rounded-xl border border-line bg-surface2"
            :class="status === 'down' ? 'text-down' : status === 'pending' ? 'text-pending' : status === 'paused' ? 'text-faint' : 'text-accent'">
            <VIcon :name="kindIcon(m.kind)" :size="24" />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
              <h1 class="truncate font-mono text-h1 text-fg">{{ m.name }}</h1>
              <StatePill :tone="statusTone[status]" :label="statusLabel[status]" />
            </div>
            <p class="mt-1 text-sm text-muted">
              {{ m.workspace }} · {{ KINDS[m.kind] || m.kind }} · check every {{ m.interval_secs }}s
              <span v-if="status === 'up' || status === 'down'"> · since {{ dur(m.since) }} ago</span>
            </p>
            <p class="mt-1 min-w-0 truncate font-mono text-xs text-faint" v-tip="m.kind === 'push' ? pushUrl : m.target">{{ m.kind === 'push' ? pushUrl : m.target }}</p>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <button @click="openEdit" class="flex items-center gap-1.5 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm font-medium text-fg hover:border-accent/50"><VIcon name="settings" :size="16" /> Edit</button>
            <button @click="removeMonitor" class="flex items-center gap-1.5 rounded-lg border border-down/35 px-3 py-1.5 text-sm font-medium text-down hover:bg-down/10"><VIcon name="trash" :size="16" /> Delete</button>
          </div>
        </div>
      </section>

      <!-- KPI strip -->
      <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div v-for="u in [{ k: 'uptime_24h', l: 'Uptime · 24h' }, { k: 'uptime_7d', l: 'Uptime · 7d' }, { k: 'uptime_30d', l: 'Uptime · 30d' }]" :key="u.k"
          class="rounded-xl border border-line bg-surface px-4 py-3">
          <div class="text-micro uppercase tracking-wider text-faint">{{ u.l }}</div>
          <div class="mt-1 font-mono text-metric tabular-nums" :class="m[u.k] == null ? 'text-faint' : m[u.k] >= 99 ? 'text-ok' : m[u.k] >= 95 ? 'text-warn' : 'text-down'">{{ pct(m[u.k]) }}</div>
        </div>
        <div class="rounded-xl border border-line bg-surface px-4 py-3">
          <div class="text-micro uppercase tracking-wider text-faint">Latency</div>
          <div class="mt-1 font-mono text-metric tabular-nums text-fg">{{ m.latency_ms != null ? m.latency_ms : '—' }}<span v-if="m.latency_ms != null" class="text-base font-normal text-faint"> ms</span></div>
          <div class="mt-0.5 text-micro text-faint">most recent check</div>
        </div>
      </div>

      <!-- alert rules covering this service -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <div class="flex items-center gap-2 border-b border-line2 bg-head px-4 py-2.5">
          <VIcon name="bell" :size="16" class="text-faint" />
          <h2 class="text-xs font-extrabold uppercase tracking-wide text-fg">Alert rules</h2>
          <span class="rounded-pill bg-surface2 px-2 py-0.5 text-micro text-muted">{{ rules.length }}</span>
        </div>
        <div class="p-4">
          <p v-if="!rules.length" class="text-sm text-faint">No alert rules cover this service. <RouterLink :to="{ name: 'alerts', query: nsq }" class="text-accent hover:underline">Add one</RouterLink>.</p>
          <div v-else class="flex flex-wrap gap-2">
            <RouterLink v-for="r in rules" :key="r.id" :to="{ name: 'alerts', query: { ...nsq, rule: r.id } }"
              class="inline-flex items-center gap-2 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-xs hover:border-accent/50">
              <span class="h-1.5 w-1.5 rounded-full" :class="r.firing === true ? 'bg-down' : r.firing === false ? 'bg-ok' : 'bg-faint'"></span>
              <span class="text-fg">{{ r.scope_kind === 'all_services' ? 'All services in workspace' : 'This service' }}</span>
              <span v-if="!r.enabled" class="text-faint">· off</span>
            </RouterLink>
          </div>
        </div>
      </div>

      <!-- range + charts -->
      <div class="flex items-center gap-2">
        <h2 class="text-h2 text-fg">History</h2>
        <div class="ml-auto flex gap-1">
          <button v-for="r in RANGES" :key="r.v" @click="range = r.v"
            class="rounded-md border px-2.5 py-1 text-xs" :class="range === r.v ? 'border-accent/60 bg-accent/10 text-accent' : 'border-line bg-surface2 text-muted hover:text-fg'">{{ r.label }}</button>
        </div>
      </div>

      <!-- up/down strip -->
      <MonitorHeartbeatBar :up="hb.up" />

      <!-- latency chart -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <div class="flex items-center gap-2 border-b border-line2 bg-head px-4 py-2.5">
          <VIcon name="latency" :size="16" class="text-faint" />
          <h2 class="text-xs font-extrabold uppercase tracking-wide text-fg">Response time</h2>
        </div>
        <div class="p-4">
          <UplotChart v-if="hb.t.length" :time="hb.t" :series="latencySeries" unit="ms" :height="180" :span-seconds="spanSeconds" :sync-key="'mon:' + id" />
          <p v-else class="text-sm text-faint">No latency data in this range yet.</p>
        </div>
      </div>

      <!-- down history / incidents -->
      <MonitorIncidentsList :incidents="incidents" :ev-time="evTime" :dur-txt="durTxt" />

      <!-- last request/response -->
      <MonitorDebugCard v-if="debug" :debug="debug" :fmt-debug="fmtDebug" :copy="copy" />
    </div>
  </AppShell>
</template>
