<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { minLoad } from '../lib/minLoad'
import UplotChart from '../components/UplotChart.vue'
import MonitorMetaCard from '../components/MonitorMetaCard.vue'
import MonitorHeartbeatBar from '../components/MonitorHeartbeatBar.vue'
import MonitorIncidentsList from '../components/MonitorIncidentsList.vue'
import MonitorDebugCard from '../components/MonitorDebugCard.vue'
import { api } from '../lib/api'

const route = useRoute()
const id = route.params.id

const m = ref(null)
const hb = ref({ t: [], latency: [], up: [] })
const events = ref([])
const debug = ref(null)
const rules = ref([]) // alert rules covering this service (own + namespace-wide)
const range = ref('24h')
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
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
const statusColor = { up: 'text-accent', down: 'text-red-500', paused: 'text-faint', pending: 'text-muted' }
const dotColor = { up: 'bg-accent', down: 'bg-red-500', paused: 'bg-faint', pending: 'bg-faint' }

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
  <AppShell :title="m?.name || 'Service'" hide-title>
    <template #title-after>
      <nav class="flex items-center gap-2 text-lg font-semibold">
        <RouterLink :to="{ name: 'monitors' }" class="text-muted hover:text-accent">Services</RouterLink>
        <span class="text-faint">›</span>
        <span class="truncate text-fg">{{ m?.name || 'Service' }}</span>
      </nav>
    </template>

    <div v-if="err" class="rounded-xl border border-line bg-surface p-6 text-center text-rose-400">{{ err }}</div>
    <PageLoader v-else-if="!m" />
    <div v-else class="space-y-5">
      <!-- header -->
      <MonitorMetaCard :m="m" :status="status" :status-label="statusLabel" :status-color="statusColor" :dot-color="dotColor" :dur="dur" :push-url="pushUrl" />

      <!-- uptime cards -->
      <div class="grid grid-cols-3 gap-3">
        <div v-for="u in [{ k: 'uptime_24h', l: '24 hours' }, { k: 'uptime_7d', l: '7 days' }, { k: 'uptime_30d', l: '30 days' }]" :key="u.k"
          class="rounded-xl border border-line bg-surface p-4">
          <div class="text-[11px] uppercase tracking-wider text-faint">Uptime · {{ u.l }}</div>
          <div class="mt-1 text-2xl font-semibold tabular-nums" :class="m[u.k] == null ? 'text-faint' : m[u.k] >= 99 ? 'text-accent' : m[u.k] >= 95 ? 'text-amber-400' : 'text-red-400'">{{ pct(m[u.k]) }}</div>
        </div>
      </div>

      <!-- alert rules covering this service -->
      <div class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-faint">
          Alert rules <span class="rounded-full bg-surface2 px-2 py-0.5 text-[10px]">{{ rules.length }}</span>
        </div>
        <p v-if="!rules.length" class="text-xs text-faint">No alert rules cover this service. <RouterLink :to="{ name: 'alerts', query: nsq }" class="text-accent hover:underline">Add one</RouterLink>.</p>
        <div v-else class="flex flex-wrap gap-2">
          <RouterLink v-for="r in rules" :key="r.id" :to="{ name: 'alerts', query: { ...nsq, rule: r.id } }"
            class="inline-flex items-center gap-2 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-xs hover:border-accent/50">
            <span class="h-1.5 w-1.5 rounded-full" :class="r.firing === true ? 'bg-red-500' : r.firing === false ? 'bg-accent' : 'bg-faint'"></span>
            <span class="text-fg">{{ r.scope_kind === 'all_services' ? 'All services in namespace' : 'This service' }}</span>
            <span v-if="!r.enabled" class="text-faint">· off</span>
          </RouterLink>
        </div>
      </div>

      <!-- range + charts -->
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-semibold text-fg">History</h2>
        <div class="ml-auto flex gap-1">
          <button v-for="r in RANGES" :key="r.v" @click="range = r.v"
            class="rounded-md border px-2.5 py-1 text-xs" :class="range === r.v ? 'border-accent/60 bg-accent/10 text-accent' : 'border-line bg-surface2 text-muted hover:text-fg'">{{ r.label }}</button>
        </div>
      </div>

      <!-- up/down strip -->
      <MonitorHeartbeatBar :up="hb.up" />

      <!-- latency chart -->
      <div class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Response time</div>
        <UplotChart v-if="hb.t.length" :time="hb.t" :series="latencySeries" unit="ms" :height="180" :span-seconds="spanSeconds" :sync-key="'mon:' + id" />
        <p v-else class="text-xs text-faint">No latency data in this range yet.</p>
      </div>

      <!-- down history / incidents -->
      <MonitorIncidentsList :incidents="incidents" :ev-time="evTime" :dur-txt="durTxt" />

      <!-- last request/response -->
      <MonitorDebugCard v-if="debug" :debug="debug" :fmt-debug="fmtDebug" :copy="copy" />
    </div>
  </AppShell>
</template>
