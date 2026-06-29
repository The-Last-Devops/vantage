<script setup>
// Overview / Dashboard — attention-first landing that summarises the whole estate:
// open incidents lead, then KPIs across infra + services + alerts, then the recent
// event stream and the fleet CPU trend. Aggregates across all selected namespaces
// (?ns=a,b; empty = all), like Systems.vue.
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import IncidentList from '../components/IncidentList.vue'
import EventStream from '../components/EventStream.vue'
import FleetCharts from '../components/FleetCharts.vue'
import VIcon from '../components/VIcon.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { online, hostState, worstReason, ago, pct, DEFAULT_THR } from '../lib/triage'

const route = useRoute()
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const inNs = (s) => selectedNs.value.length === 0 || selectedNs.value.includes(s.namespace)

const systems = ref([])
const monitors = ref([]) // service checks
const events = ref([]) // recent service status changes
const thresholds = ref({}) // namespace name -> thresholds
const namespaces = ref([])
const alerts = ref([]) // firing alert rows across selected namespaces
const fleet = ref(null)
let timer = null

const thrOf = (s) => thresholds.value[s.namespace] || DEFAULT_THR
const hosts = computed(() => systems.value.filter(inNs))
const nsMonitors = computed(() => monitors.value.filter(inNs))
function avg(arr, f) { const v = arr.map(f).filter((x) => x != null); return v.length ? Math.round(v.reduce((a, b) => a + b, 0) / v.length) : null }

const summary = computed(() => {
  let up = 0, down = 0, warn = 0
  for (const s of hosts.value) {
    if (online(s)) up++
    const st = hostState(s, thrOf(s))
    if (st === 'down') down++
    else if (st === 'warn') warn++
  }
  return {
    total: hosts.value.length, up, down, warn,
    cpu: avg(hosts.value.filter(online), (s) => s.cpu_percent),
  }
})

// Service summary (a check is "active" unless paused/disabled).
const svc = computed(() => {
  let active = 0, up = 0, down = 0
  for (const m of nsMonitors.value) {
    if (!m.enabled) continue
    active++
    if (m.up === true) up++
    else if (m.up === false) down++
  }
  return { active, up, down }
})

const firing = computed(() => alerts.value.filter((a) => a.enabled && a.firing === true))

// Incidents = each down/warn host + each down service + each firing alert.
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
  for (const m of nsMonitors.value) {
    if (m.enabled && m.up === false) {
      out.push({ id: 'm:' + m.id, tone: 'down', host: m.name, reason: 'service check down', ns: m.namespace, systemId: null })
    }
  }
  for (const a of firing.value) {
    const dur = a.since ? ' · ' + ago(a.since) : ''
    out.push({ id: 'a:' + a.id, tone: 'down', host: a.target_name || 'alert', reason: `${condText(a)} firing${dur}`, ns: a.namespace, systemId: a.system_id || null })
  }
  return out.sort((x, y) => (x.tone === y.tone ? x.host.localeCompare(y.host) : x.tone === 'down' ? -1 : 1))
})

const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function condText(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor' || a.target_kind === 'all_services') return 'service down'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return 'alert'
}

// ---- recent service events (reuses Monitors' shaping) ----
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

// ---- fleet avg CPU trend (24h) ----
const FSPAN = 86400
const trendCharts = computed(() => {
  const f = fleet.value
  if (!f || !f.t || !f.t.length) return []
  const visible = new Set(hosts.value.map((s) => s.name))
  const series = (f.cpu || []).filter((s) => visible.has(s.name))
  if (!series.length) return []
  const data = f.t.map((_, i) => {
    const vals = series.map((s) => s.data[i]).filter((v) => v != null)
    return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null
  })
  return [{ title: 'Avg CPU · last 24h', unit: '%', series: [{ name: 'fleet avg', color: 'rgb(var(--accent))', data }] }]
})

const { loaded, reload: load } = useCached({
  key: () => 'overview:' + selectedNs.value.join(','),
  load: async () => {
    const nss = namespaces.value
    const sel = selectedNs.value.length ? nss.filter((n) => selectedNs.value.includes(n.name)) : nss
    const [sys, mons, evs, thr, fl, alertLists] = await Promise.all([
      api.get('/api/systems').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/events?range=7d').catch(() => []),
      api.get('/api/thresholds').catch(() => []),
      api.get('/api/fleet?range=24h').catch(() => null),
      Promise.all(sel.map((n) =>
        api.get(`/api/namespaces/${n.id}/alerts`)
          .then((rows) => rows.map((x) => ({ ...x, namespace: n.name })))
          .catch(() => []),
      )),
    ])
    const tm = {}; for (const x of thr) tm[x.namespace] = x
    const seen = new Set()
    const al = alertLists.flat().filter((a) => !seen.has(a.id) && seen.add(a.id))
    return { systems: sys, monitors: mons, events: evs, thresholds: tm, alerts: al, fleet: fl }
  },
  apply: (d) => { systems.value = d.systems; monitors.value = d.monitors; events.value = d.events; thresholds.value = d.thresholds; alerts.value = d.alerts; fleet.value = d.fleet },
})
watch(() => route.query.ns, load)
onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  await load()
  timer = setInterval(load, 10000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Overview">
    <template #title-after>
      <span class="text-xs text-muted">
        {{ summary.total }} hosts · {{ svc.active }} services
        <span v-if="summary.down" class="text-down"> · {{ summary.down }} down</span>
        <span v-if="firing.length" class="text-down"> · {{ firing.length }} firing</span>
      </span>
    </template>

    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <!-- 1) Needs attention FIRST -->
      <IncidentList :incidents="incidents" />

      <!-- 2) KPI strip — infra + services + alerts -->
      <section class="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="text-xs uppercase tracking-wider text-muted">Hosts up</div>
          <div class="mt-1 font-mono text-metric font-extrabold tabular-nums text-ok">{{ summary.up }}<span class="text-h2 text-faint">/{{ summary.total }}</span></div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="text-xs uppercase tracking-wider text-muted">Services up</div>
          <div class="mt-1 font-mono text-metric font-extrabold tabular-nums" :class="svc.down ? 'text-down' : 'text-ok'">{{ svc.up }}<span class="text-h2 text-faint">/{{ svc.active }}</span></div>
        </div>
        <div class="rounded-xl border p-4" :class="firing.length ? 'border-down/38 bg-down/12' : 'border-line bg-surface'">
          <div class="text-xs uppercase tracking-wider text-muted">Alerts firing</div>
          <div class="mt-1 font-mono text-metric font-extrabold tabular-nums" :class="firing.length ? 'text-down' : 'text-fg'">{{ firing.length }}</div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="text-xs uppercase tracking-wider text-muted">Avg CPU</div>
          <div class="mt-1 font-mono text-metric font-extrabold tabular-nums text-fg">{{ summary.cpu ?? '—' }}<span class="text-h2 text-faint">%</span></div>
        </div>
      </section>

      <!-- 3) Recent events + fleet trend -->
      <section class="grid gap-5 lg:grid-cols-2">
        <EventStream :events="shownEvents" :ev-time="evTime" :ev-message="evMessage" :state-dur="stateDur" :fmt-dur="fmtDur" class="h-fit" />
        <div class="h-fit rounded-xl border border-line bg-surface p-4">
          <div class="mb-2 flex items-center gap-2">
            <VIcon name="metrics" :size="16" class="text-muted" />
            <h2 class="text-h2 font-semibold text-fg">Fleet trend</h2>
          </div>
          <FleetCharts v-if="trendCharts.length" :charts="trendCharts" :time="fleet?.t || []" :span-seconds="FSPAN" sync-key="overview-trend" />
          <p v-else class="py-6 text-center text-xs text-muted">No metrics yet.</p>
        </div>
      </section>
    </div>
  </AppShell>
</template>
