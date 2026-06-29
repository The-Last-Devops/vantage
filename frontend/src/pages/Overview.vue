<script setup>
// Overview / Dashboard (Rancher-style): basic estate info up top — summary counters,
// capacity bars, and a "needs attention" banner — then the recent events as a paged,
// length-capped table at the bottom. Aggregates across selected namespaces (?ns=).
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import IncidentList from '../components/IncidentList.vue'
import StatePill from '../components/StatePill.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { online, hostState, worstReason, ago, pct, DEFAULT_THR, STATE_RANK } from '../lib/triage'

const route = useRoute()
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const inNs = (s) => selectedNs.value.length === 0 || selectedNs.value.includes(s.namespace)

const systems = ref([])
const monitors = ref([])
const events = ref([])
const thresholds = ref({})
const namespaces = ref([])
const alerts = ref([])
let timer = null

const thrOf = (s) => thresholds.value[s.namespace] || DEFAULT_THR
const hosts = computed(() => systems.value.filter(inNs))
const nsMonitors = computed(() => monitors.value.filter(inNs))
const monName = computed(() => { const m = {}; for (const x of monitors.value) m[x.id] = x.name; return m })
function avg(arr, f) { const v = arr.map(f).filter((x) => x != null); return v.length ? Math.round(v.reduce((a, b) => a + b, 0) / v.length) : null }

const summary = computed(() => {
  let up = 0, down = 0, warn = 0, crit = 0
  for (const s of hosts.value) {
    if (online(s)) up++
    const st = hostState(s, thrOf(s))
    if (st === 'down') down++
    else if (st === 'crit') crit++
    else if (st === 'warn') warn++
  }
  const onl = hosts.value.filter(online)
  return {
    total: hosts.value.length, up, down, warn, crit,
    cpu: avg(onl, (s) => s.cpu_percent),
    mem: avg(onl, (s) => pct(s.mem_used, s.mem_total)),
    disk: avg(onl, (s) => pct(s.disk_used, s.disk_total)),
  }
})
const svc = computed(() => {
  let active = 0, up = 0, down = 0
  for (const m of nsMonitors.value) { if (!m.enabled) continue; active++; if (m.up === true) up++; else if (m.up === false) down++ }
  return { active, up, down }
})
const firing = computed(() => alerts.value.filter((a) => a.enabled && a.firing === true))

// capacity bar tone by threshold (<70 ok / 70-90 warn / >90 down)
const capTone = (v) => (v == null ? 'bg-track' : v > 90 ? 'bg-down' : v >= 70 ? 'bg-warn' : 'bg-ok')
const capacity = computed(() => [
  { k: 'CPU', v: summary.value.cpu },
  { k: 'Memory', v: summary.value.mem },
  { k: 'Disk', v: summary.value.disk },
])

const incidents = computed(() => {
  const out = []
  for (const s of hosts.value) {
    const st = hostState(s, thrOf(s))
    if (st === 'ok') continue
    const reason = online(s) ? `${worstReason(s, thrOf(s)) || 'over threshold'} · ${ago(s.last_seen)}` : `offline · ${ago(s.last_seen)}`
    out.push({ id: 'h:' + s.id, tone: st, host: s.name, reason, ns: s.namespace, systemId: s.id })
  }
  for (const m of nsMonitors.value) {
    if (m.enabled && m.up === false) out.push({ id: 'm:' + m.id, tone: 'down', host: m.name, reason: 'service check down', ns: m.namespace, systemId: null })
  }
  for (const a of firing.value) {
    const dur = a.since ? ' · ' + ago(a.since) : ''
    out.push({ id: 'a:' + a.id, tone: 'down', host: a.target_name || 'alert', reason: `${condText(a)} firing${dur}`, ns: a.namespace, systemId: a.system_id || null })
  }
  return out.sort((x, y) => (STATE_RANK[x.tone] ?? 9) - (STATE_RANK[y.tone] ?? 9) || x.host.localeCompare(y.host))
})
const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function condText(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor' || a.target_kind === 'all_services') return 'service down'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return 'alert'
}

// ---- events table (length-capped + paged) ----
const shownEvents = computed(() => {
  if (!selectedNs.value.length) return events.value
  const ids = new Set(nsMonitors.value.map((m) => m.id))
  return events.value.filter((e) => ids.has(e.monitor_id))
})
const PAGE = 8
const evPage = ref(1)
const evPages = computed(() => Math.max(1, Math.ceil(shownEvents.value.length / PAGE)))
const evRows = computed(() => {
  const p = Math.min(evPage.value, evPages.value)
  return shownEvents.value.slice((p - 1) * PAGE, p * PAGE)
})
watch([() => route.query.ns, evPages], () => { if (evPage.value > evPages.value) evPage.value = 1 })
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })

const { loaded, reload: load } = useCached({
  key: () => 'overview:' + selectedNs.value.join(','),
  load: async () => {
    const nss = namespaces.value
    const sel = selectedNs.value.length ? nss.filter((n) => selectedNs.value.includes(n.name)) : nss
    const [sys, mons, evs, thr, alertLists] = await Promise.all([
      api.get('/api/systems').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/events?range=7d').catch(() => []),
      api.get('/api/thresholds').catch(() => []),
      Promise.all(sel.map((n) =>
        api.get(`/api/namespaces/${n.id}/alerts`).then((rows) => rows.map((x) => ({ ...x, namespace: n.name }))).catch(() => []))),
    ])
    const tm = {}; for (const x of thr) tm[x.namespace] = x
    const seen = new Set()
    const al = alertLists.flat().filter((a) => !seen.has(a.id) && seen.add(a.id))
    return { systems: sys, monitors: mons, events: evs, thresholds: tm, alerts: al }
  },
  apply: (d) => { systems.value = d.systems; monitors.value = d.monitors; events.value = d.events; thresholds.value = d.thresholds; alerts.value = d.alerts },
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
      <span class="text-xs text-muted">{{ summary.total }} hosts · {{ svc.active }} services</span>
    </template>

    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-5">
      <!-- BASIC INFO: summary counters -->
      <section class="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="flex flex-wrap items-baseline gap-1.5"><span class="font-mono text-metric font-extrabold tabular-nums text-fg">{{ summary.total }}</span><span v-if="summary.down" class="rounded bg-down/15 px-1.5 text-xs font-semibold text-down">{{ summary.down }} down</span><span v-if="summary.crit" class="rounded bg-crit/15 px-1.5 text-xs font-semibold text-crit">{{ summary.crit }} critical</span></div>
          <div class="mt-1 text-sm text-muted">Hosts <span class="text-faint">· {{ summary.up }} up</span></div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="flex items-baseline gap-2"><span class="font-mono text-metric font-extrabold tabular-nums text-fg">{{ svc.active }}</span><span v-if="svc.down" class="rounded bg-down/15 px-1.5 text-xs font-semibold text-down">{{ svc.down }} down</span></div>
          <div class="mt-1 text-sm text-muted">Services <span class="text-faint">· {{ svc.up }} up</span></div>
        </div>
        <div class="rounded-xl border p-4" :class="firing.length ? 'border-down/38 bg-down/12' : 'border-line bg-surface'">
          <div class="font-mono text-metric font-extrabold tabular-nums" :class="firing.length ? 'text-down' : 'text-fg'">{{ firing.length }}</div>
          <div class="mt-1 text-sm text-muted">Alerts firing</div>
        </div>
        <div class="rounded-xl border border-line bg-surface p-4">
          <div class="font-mono text-metric font-extrabold tabular-nums text-fg">{{ namespaces.length }}</div>
          <div class="mt-1 text-sm text-muted">Namespaces</div>
        </div>
      </section>

      <!-- BASIC INFO: capacity -->
      <section class="rounded-xl border border-line bg-surface p-4">
        <h2 class="mb-3 text-h2 font-semibold text-fg">Capacity</h2>
        <div class="grid gap-4 sm:grid-cols-3">
          <div v-for="c in capacity" :key="c.k">
            <div class="flex items-baseline justify-between text-sm"><span class="text-muted">{{ c.k }}</span><span class="font-mono tabular-nums text-fg">{{ c.v ?? '—' }}<span class="text-faint">%</span></span></div>
            <div class="mt-1.5 h-2 overflow-hidden rounded bg-track"><div class="h-full rounded" :class="capTone(c.v)" :style="{ width: (c.v ?? 0) + '%' }"></div></div>
          </div>
        </div>
      </section>

      <!-- NEEDS ATTENTION — below the basics, height-capped so it can't take over -->
      <IncidentList v-if="incidents.length" :incidents="incidents" scroll />

      <!-- EVENTS: length-capped + paged table -->
      <section class="overflow-hidden rounded-xl border border-line bg-surface">
        <div class="flex items-center gap-2 border-b border-line2 bg-head px-4 py-2.5">
          <h2 class="text-h2 font-semibold text-fg">Events</h2>
          <span class="font-mono text-xs text-faint">{{ shownEvents.length }} in 7d</span>
          <RouterLink :to="{ name: 'events', query: route.query.ns ? { ns: route.query.ns } : {} }" class="ml-auto text-sm text-accent hover:underline">Full events list →</RouterLink>
        </div>
        <div class="overflow-x-auto">
          <table class="w-full border-collapse text-sm">
            <thead>
              <tr class="border-b border-line2 bg-head text-xs uppercase tracking-wide text-fg">
                <th class="px-4 py-2 text-left font-extrabold">Status</th>
                <th class="px-4 py-2 text-left font-extrabold">Service</th>
                <th class="px-4 py-2 text-left font-extrabold">Message</th>
                <th class="px-4 py-2 text-right font-extrabold">Time</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="!shownEvents.length"><td colspan="4" class="px-4 py-10 text-center text-muted">No events.</td></tr>
              <tr v-for="(e, i) in evRows" :key="e.id || i" class="border-b border-line last:border-0 hover:bg-hover">
                <td class="px-4 py-2.5"><StatePill :tone="e.up ? 'ok' : 'down'" :label="e.up ? 'Up' : 'Down'" /></td>
                <td class="px-4 py-2.5 font-mono text-accent">{{ monName[e.monitor_id] || '—' }}</td>
                <td class="px-4 py-2.5 text-fg">{{ e.message || (e.up ? 'Recovered' : 'Down') }}</td>
                <td class="px-4 py-2.5 text-right font-mono tabular-nums text-faint">{{ evTime(e.at) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <!-- pagination -->
        <div v-if="evPages > 1" class="flex items-center justify-end gap-3 border-t border-line px-4 py-2.5 text-sm">
          <span class="font-mono text-xs text-faint">Page {{ Math.min(evPage, evPages) }} / {{ evPages }}</span>
          <button :disabled="evPage <= 1" @click="evPage = Math.max(1, evPage - 1)" class="rounded-lg border border-line2 bg-surface2 px-2.5 py-1 text-muted hover:text-fg disabled:opacity-40">Prev</button>
          <button :disabled="evPage >= evPages" @click="evPage = Math.min(evPages, evPage + 1)" class="rounded-lg border border-line2 bg-surface2 px-2.5 py-1 text-muted hover:text-fg disabled:opacity-40">Next</button>
        </div>
      </section>
    </div>
  </AppShell>
</template>
