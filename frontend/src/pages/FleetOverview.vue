<script setup>
// Fleet overview — a war-room board: problems on top, a health heatmap + services
// on the left, incidents + top-load on the right. Aggregates across all selected
// namespaces (?ns=a,b; empty = all), like Systems.vue.
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import IncidentList from '../components/IncidentList.vue'
import FleetHeatmap from '../components/FleetHeatmap.vue'
import VIcon from '../components/VIcon.vue'
import StatePill from '../components/StatePill.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { online, hostState, worstReason, ago, DEFAULT_THR, STATE_RANK } from '../lib/triage'

const route = useRoute()
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const inNs = (s) => selectedNs.value.length === 0 || selectedNs.value.includes(s.namespace)

const systems = ref([])
const monitors = ref([])
const thresholds = ref({})
const namespaces = ref([])
const alerts = ref([])
const lastUpdate = ref(Date.now())
const nowTick = ref(Date.now())
let timer = null
let tick = null

const thrOf = (s) => thresholds.value[s.namespace] || DEFAULT_THR
const hosts = computed(() => systems.value.filter(inNs))
const svcs = computed(() => monitors.value.filter(inNs))
const firing = computed(() => alerts.value.filter((a) => a.enabled && a.firing === true))

const crumbNs = computed(() => selectedNs.value.length ? selectedNs.value.join(', ') : 'all namespaces')
const updatedAgo = computed(() => Math.max(0, Math.round((nowTick.value - lastUpdate.value) / 1000)))

// ---- KPIs ----
const kpis = computed(() => {
  let up = 0, down = 0, warn = 0, crit = 0
  for (const s of hosts.value) {
    if (online(s)) up++
    const st = hostState(s, thrOf(s))
    if (st === 'down') down++
    else if (st === 'crit') crit++
    else if (st === 'warn') warn++
  }
  let svcUp = 0, svcTotal = 0
  for (const m of svcs.value) {
    if (!m.enabled) continue
    svcTotal++
    if (m.up === true) svcUp++
  }
  return { up, total: hosts.value.length, down, warn, crit, svcUp, svcTotal, firing: firing.value.length }
})

// ---- heatmap groups (by namespace) ----
const groups = computed(() => {
  const by = new Map()
  for (const s of hosts.value) {
    const ns = s.namespace || '—'
    if (!by.has(ns)) by.set(ns, [])
    let state = hostState(s, thrOf(s))
    by.get(ns).push({ id: s.id, name: s.name, state })
  }
  return [...by.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([name, list]) => ({ name, hosts: list.sort((a, b) => a.name.localeCompare(b.name)) }))
})

// ---- services (with sparkline points + uptime%) ----
const SVC_STATE = { up: ['ok', 'Up'], down: ['down', 'Down'], pending: ['warn', 'Pending'], paused: ['muted', 'Paused'] }
const stateKey = (m) => (!m.enabled ? 'paused' : m.up === true ? 'up' : m.up === false ? 'down' : 'pending')
const upPct = (m) => (m.recent && m.recent.length ? Math.round((m.recent.filter(Boolean).length / m.recent.length) * 100) : null)
// sparkline polyline points from the recent boolean beats (up=1, down=0)
function spark(recent) {
  const beats = (recent || []).slice(-24)
  if (!beats.length) return ''
  const w = 60, h = 16, step = beats.length > 1 ? w / (beats.length - 1) : 0
  return beats.map((b, i) => `${(i * step).toFixed(1)},${(b ? 1 : h - 1).toFixed(1)}`).join(' ')
}
const svcRows = computed(() =>
  [...svcs.value]
    .sort((a, b) => (a.namespace || '').localeCompare(b.namespace || '') || a.name.localeCompare(b.name))
    .map((m) => ({ ...m, sk: stateKey(m), uptime: upPct(m), points: spark(m.recent) })),
)

// ---- top load (online hosts by CPU desc) ----
const loadRows = computed(() =>
  hosts.value
    .filter(online)
    .map((s) => ({ id: s.id, name: s.name, ns: s.namespace, cpu: Math.round(s.cpu_percent || 0) }))
    .sort((a, b) => b.cpu - a.cpu)
    .slice(0, 8),
)
const barTone = (v) => (v >= 90 ? 'bg-down' : v >= 70 ? 'bg-warn' : 'bg-ok')
const cpuText = (v) => (v >= 90 ? 'text-down' : v >= 70 ? 'text-warn' : 'text-fg')

// ---- incidents (shared with Overview) ----
const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function condText(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor' || a.target_kind === 'all_services') return 'service down'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return 'alert'
}
const incidents = computed(() => {
  const out = []
  for (const s of hosts.value) {
    const st = hostState(s, thrOf(s))
    if (st === 'ok') continue
    const reason = online(s)
      ? `${worstReason(s, thrOf(s)) || 'over threshold'} · ${ago(s.last_seen)}`
      : `offline · ${ago(s.last_seen)}`
    out.push({ id: 'h:' + s.id, tone: st, host: s.name, reason, ns: s.namespace, systemId: s.id })
  }
  for (const a of firing.value) {
    const dur = a.since ? ' · ' + ago(a.since) : ''
    out.push({ id: 'a:' + a.id, tone: 'down', host: a.target_name || 'alert', reason: `${condText(a)} firing${dur}`, ns: a.namespace, systemId: a.system_id || null })
  }
  return out.sort((x, y) => (STATE_RANK[x.tone] ?? 9) - (STATE_RANK[y.tone] ?? 9) || x.host.localeCompare(y.host))
})

const { loaded, reload: load } = useCached({
  key: () => 'fleet-overview:' + selectedNs.value.join(','),
  load: async () => {
    const nss = namespaces.value
    const sel = selectedNs.value.length ? nss.filter((n) => selectedNs.value.includes(n.name)) : nss
    const [sys, mons, thr, alertLists] = await Promise.all([
      api.get('/api/systems').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/thresholds').catch(() => []),
      Promise.all(sel.map((n) =>
        api.get(`/api/namespaces/${n.id}/alerts`)
          .then((rows) => rows.map((x) => ({ ...x, namespace: n.name })))
          .catch(() => []),
      )),
    ])
    const tm = {}; for (const x of thr) tm[x.namespace] = x
    const seen = new Set()
    const al = alertLists.flat().filter((a) => !seen.has(a.id) && seen.add(a.id))
    return { systems: sys, monitors: mons, thresholds: tm, alerts: al }
  },
  apply: (d) => { systems.value = d.systems; monitors.value = d.monitors; thresholds.value = d.thresholds; alerts.value = d.alerts; lastUpdate.value = Date.now() },
})
watch(() => route.query.ns, load)
onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  await load()
  timer = setInterval(load, 10000)
  tick = setInterval(() => { nowTick.value = Date.now() }, 1000)
})
onUnmounted(() => { clearInterval(timer); clearInterval(tick) })
</script>

<template>
  <AppShell hide-title>
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-4">
      <!-- top bar -->
      <div class="flex flex-wrap items-center gap-3">
        <div class="flex items-center gap-2">
          <VIcon name="fleet" :size="20" class="text-accent" />
          <h1 class="text-h1 font-semibold text-fg">Fleet</h1>
          <span class="text-xs text-muted">· {{ crumbNs }}</span>
        </div>
        <div class="ml-auto flex items-center gap-1.5 text-xs text-muted">
          <span class="relative flex h-2 w-2">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-ok opacity-75"></span>
            <span class="relative inline-flex h-2 w-2 rounded-full bg-ok"></span>
          </span>
          live · updated {{ updatedAgo }}s ago
        </div>
      </div>

      <!-- KPI row (scrolls on mobile) -->
      <div class="-mx-1 overflow-x-auto px-1">
        <div class="flex min-w-[640px] gap-3">
          <div class="flex-1 rounded-xl border border-line bg-surface p-3">
            <div class="text-micro uppercase tracking-wider text-muted">Hosts up</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums text-ok">{{ kpis.up }}<span class="text-h2 text-faint">/{{ kpis.total }}</span></div>
          </div>
          <div class="flex-1 rounded-xl border p-3" :class="kpis.down ? 'border-down/38 bg-down/12' : 'border-line bg-surface'">
            <div class="text-micro uppercase tracking-wider text-muted">Down</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums" :class="kpis.down ? 'text-down' : 'text-fg'">{{ kpis.down }}</div>
          </div>
          <div class="flex-1 rounded-xl border p-3" :class="kpis.crit ? 'border-crit/38 bg-crit/12' : 'border-line bg-surface'">
            <div class="text-micro uppercase tracking-wider text-muted">Critical</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums" :class="kpis.crit ? 'text-crit' : 'text-fg'">{{ kpis.crit }}</div>
          </div>
          <div class="flex-1 rounded-xl border p-3" :class="kpis.warn ? 'border-warn/38 bg-warn/12' : 'border-line bg-surface'">
            <div class="text-micro uppercase tracking-wider text-muted">Warning</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums" :class="kpis.warn ? 'text-warn' : 'text-fg'">{{ kpis.warn }}</div>
          </div>
          <div class="flex-1 rounded-xl border border-line bg-surface p-3">
            <div class="text-micro uppercase tracking-wider text-muted">Services</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums text-fg">{{ kpis.svcUp }}<span class="text-h2 text-faint">/{{ kpis.svcTotal }}</span></div>
          </div>
          <div class="flex-1 rounded-xl border p-3" :class="kpis.firing ? 'border-down/38 bg-down/12' : 'border-line bg-surface'">
            <div class="text-micro uppercase tracking-wider text-muted">Firing</div>
            <div class="mt-0.5 font-mono text-metric font-extrabold tabular-nums" :class="kpis.firing ? 'text-down' : 'text-fg'">{{ kpis.firing }}</div>
          </div>
        </div>
      </div>

      <!-- two-column board -->
      <div class="grid grid-cols-1 gap-4 lg:grid-cols-[1.55fr_1fr]">
        <!-- LEFT -->
        <div class="space-y-4">
          <FleetHeatmap :groups="groups" />

          <!-- services -->
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <div class="flex items-center gap-2 border-b border-line px-4 py-3">
              <VIcon name="service" :size="16" class="text-muted" />
              <h2 class="text-h2 font-semibold text-fg">Services</h2>
              <span class="ml-auto text-xs text-muted">{{ kpis.svcUp }}/{{ kpis.svcTotal }} up</span>
            </div>
            <div v-if="!svcRows.length" class="py-6 text-center text-xs text-muted">No services.</div>
            <ul v-else class="divide-y divide-line">
              <li v-for="m in svcRows" :key="m.id" class="flex items-center gap-3 px-4 py-2">
                <StatePill :tone="SVC_STATE[m.sk][0]" :label="SVC_STATE[m.sk][1]" />
                <RouterLink :to="{ name: 'monitor', params: { id: m.id } }" class="min-w-0 flex-1 truncate font-mono text-body text-fg hover:text-accent">{{ m.name }}</RouterLink>
                <svg v-if="m.points" width="60" height="16" viewBox="0 0 60 16" class="shrink-0" :class="m.sk === 'down' ? 'text-down' : m.sk === 'up' ? 'text-ok' : 'text-muted'" preserveAspectRatio="none">
                  <polyline :points="m.points" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
                </svg>
                <span class="w-12 shrink-0 text-right font-mono text-xs tabular-nums" :class="m.uptime == null ? 'text-faint' : m.uptime >= 99 ? 'text-ok' : m.uptime >= 90 ? 'text-warn' : 'text-down'">{{ m.uptime == null ? 'N/A' : m.uptime + '%' }}</span>
              </li>
            </ul>
          </div>
        </div>

        <!-- RIGHT -->
        <div class="space-y-4">
          <IncidentList :incidents="incidents" />

          <!-- top load -->
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <div class="flex items-center gap-2 border-b border-line px-4 py-3">
              <VIcon name="cpu" :size="16" class="text-muted" />
              <h2 class="text-h2 font-semibold text-fg">Top load</h2>
            </div>
            <div v-if="!loadRows.length" class="py-6 text-center text-xs text-muted">No online hosts.</div>
            <ul v-else class="divide-y divide-line">
              <li v-for="h in loadRows" :key="h.id" class="px-4 py-2">
                <div class="mb-1 flex items-center gap-2">
                  <RouterLink :to="{ name: 'system', params: { id: h.id } }" class="min-w-0 flex-1 truncate font-mono text-body text-fg hover:text-accent">{{ h.name }}</RouterLink>
                  <span class="shrink-0 font-mono text-xs font-semibold tabular-nums" :class="cpuText(h.cpu)">{{ h.cpu }}%</span>
                </div>
                <div class="h-1.5 overflow-hidden rounded-full bg-track">
                  <div class="h-full rounded-full" :class="barTone(h.cpu)" :style="{ width: Math.min(100, h.cpu) + '%' }"></div>
                </div>
              </li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  </AppShell>
</template>
