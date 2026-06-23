<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import UplotChart from '../components/UplotChart.vue'
import { api } from '../lib/api'

const route = useRoute()
const id = route.params.id

const m = ref(null)
const hb = ref({ t: [], latency: [], up: [] })
const debug = ref(null)
const range = ref('24h')
const err = ref('')
let timer = null

const RANGES = [
  { v: '1h', label: '1h' },
  { v: '6h', label: '6h' },
  { v: '24h', label: '24h' },
  { v: '7d', label: '7d' },
  { v: '30d', label: '30d' },
]

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
async function loadDebug() {
  try { debug.value = await api.get(`/api/monitors/${id}/debug`) } catch { debug.value = null }
}
watch(range, loadHb)

const fmtDebug = (d) => (typeof d === 'string' ? d : JSON.stringify(d, null, 2))
function copy(d, e) {
  navigator.clipboard?.writeText(fmtDebug(d))
  const b = e.target; const t = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = t), 1200)
}

onMounted(async () => {
  await Promise.all([loadMeta(), loadHb(), loadDebug()])
  timer = setInterval(() => { loadMeta(); loadHb() }, 30000)
})
onUnmounted(() => timer && clearInterval(timer))
</script>

<template>
  <AppShell :title="m?.name || 'Service'">
    <template #title-after>
      <span class="text-sm text-faint">›
        <RouterLink :to="{ name: 'monitors' }" class="hover:text-accent">Services</RouterLink>
      </span>
    </template>

    <div v-if="err" class="rounded-xl border border-line bg-surface p-6 text-center text-rose-400">{{ err }}</div>
    <div v-else-if="!m" class="text-sm text-muted">Loading…</div>
    <div v-else class="space-y-5">
      <!-- header -->
      <div class="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border border-line bg-surface p-4">
        <div class="flex items-center gap-2">
          <span class="h-2.5 w-2.5 rounded-full" :class="dotColor[status]"></span>
          <span class="text-lg font-semibold" :class="statusColor[status]">{{ statusLabel[status] }}</span>
          <span v-if="status === 'up' || status === 'down'" class="text-sm text-muted">for {{ dur(m.since) }}</span>
        </div>
        <div class="text-sm text-muted"><span class="text-faint">Type</span> {{ m.kind }}</div>
        <div class="text-sm text-muted"><span class="text-faint">Namespace</span> {{ m.namespace }}</div>
        <div class="text-sm text-muted"><span class="text-faint">Interval</span> {{ m.interval_secs }}s</div>
        <div v-if="m.latency_ms != null" class="text-sm text-muted"><span class="text-faint">Latency</span> {{ m.latency_ms }} ms</div>
        <div class="min-w-0 flex-1 truncate text-right font-mono text-xs text-muted" :title="m.kind === 'push' ? pushUrl : m.target">
          {{ m.kind === 'push' ? pushUrl : m.target }}
        </div>
      </div>

      <!-- uptime cards -->
      <div class="grid grid-cols-3 gap-3">
        <div v-for="u in [{ k: 'uptime_24h', l: '24 hours' }, { k: 'uptime_7d', l: '7 days' }, { k: 'uptime_30d', l: '30 days' }]" :key="u.k"
          class="rounded-xl border border-line bg-surface p-4">
          <div class="text-[11px] uppercase tracking-wider text-faint">Uptime · {{ u.l }}</div>
          <div class="mt-1 text-2xl font-semibold tabular-nums" :class="m[u.k] == null ? 'text-faint' : m[u.k] >= 99 ? 'text-accent' : m[u.k] >= 95 ? 'text-amber-400' : 'text-red-400'">{{ pct(m[u.k]) }}</div>
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
      <div class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Status</div>
        <div v-if="hb.up.length" class="flex h-7 gap-px overflow-hidden rounded">
          <div v-for="(u, i) in hb.up" :key="i" class="flex-1"
            :class="u == null ? 'bg-line' : u >= 1 ? 'bg-accent' : 'bg-red-500'"
            :title="u == null ? 'no data' : u >= 1 ? 'up' : 'down'"></div>
        </div>
        <p v-else class="text-xs text-faint">No heartbeats in this range yet.</p>
      </div>

      <!-- latency chart -->
      <div class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Response time</div>
        <UplotChart v-if="hb.t.length" :time="hb.t" :series="latencySeries" unit="ms" :height="180" :sync-key="'mon:' + id" />
        <p v-else class="text-xs text-faint">No latency data in this range yet.</p>
      </div>

      <!-- last request/response -->
      <div v-if="debug" class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Last request / response</div>
        <div class="grid gap-4 lg:grid-cols-2">
          <div>
            <div class="mb-1 flex items-center justify-between">
              <span class="text-xs font-medium text-accent">Last success</span>
              <button v-if="debug.ok" @click="copy(debug.ok, $event)" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
            </div>
            <pre v-if="debug.ok" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debug.ok) }}</pre>
            <p v-else class="text-xs text-faint">No successful check recorded yet.</p>
          </div>
          <div>
            <div class="mb-1 flex items-center justify-between">
              <span class="text-xs font-medium text-red-400">Last failure</span>
              <button v-if="debug.err" @click="copy(debug.err, $event)" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
            </div>
            <pre v-if="debug.err" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debug.err) }}</pre>
            <p v-else class="text-xs text-faint">No failure recorded.</p>
          </div>
        </div>
      </div>
    </div>
  </AppShell>
</template>
