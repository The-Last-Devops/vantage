<script setup>
// Overview / Dashboard: system health only (issue #4). Two tiers of tile:
//   · ACTION tiles — a number that, when non-zero, tells you what to do next (hosts
//     down / critical / warning, services down, alerts firing). Large, up top, tinted
//     only when something needs attention, each linking to the filtered list.
//   · INFO tiles — totals, nodes, pods, cores, uptime. A compact strip below; they
//     set the scale but never ask for anything.
// Account, backup, database and membership are Settings' business and are not here.
// Aggregates across the selected workspaces (?ws=).
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { online, hostState, DEFAULT_THR, ago } from '../lib/triage'

const route = useRoute()
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const nsq = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
const inWs = (s) => selectedWs.value.length === 0 || selectedWs.value.includes(s.workspace)

const systems = ref([])
const monitors = ref([])
const thresholds = ref({})
const workspaces = ref([])
const alerts = ref([])
const clusterSums = ref({}) // k8s-cluster system id -> kube/summary roll-up
let timer = null

const thrOf = (s) => thresholds.value[s.workspace] || DEFAULT_THR
// Hosts excludes k8s clusters — those are counted in their own Clusters tiles.
const hosts = computed(() => systems.value.filter(inWs).filter((s) => s.kind !== 'k8s-cluster'))
const clusterList = computed(() => systems.value.filter(inWs).filter((s) => s.kind === 'k8s-cluster'))
const clusterAgg = computed(() => {
  const a = { total: clusterList.value.length, online: 0, nodes: 0, pods: 0, cpu: 0 }
  for (const c of clusterList.value) {
    if (online(c)) a.online++
    const s = clusterSums.value[c.id]
    if (s) { a.nodes += s.nodes; a.pods += s.pods_running; a.cpu += s.cpu_millicores }
  }
  return a
})
const wsMonitors = computed(() => monitors.value.filter(inWs).filter((m) => m.enabled))

// ---- counts ----
const host = computed(() => {
  let up = 0, down = 0, crit = 0, warn = 0, oldest = null
  for (const s of hosts.value) {
    if (online(s)) up++
    const st = hostState(s, thrOf(s))
    if (st === 'down') { down++; if (s.last_seen && (!oldest || s.last_seen < oldest)) oldest = s.last_seen }
    else if (st === 'crit') crit++
    else if (st === 'warn') warn++
  }
  // "Down" is a word; "down for 2d 5h" is a decision (issue #5). The longest outage
  // is the one that matters most, so the tile carries that.
  return { total: hosts.value.length, up, down, crit, warn, longest: oldest ? ago(oldest) : '' }
})
// A service has no "down since" on the wire; its 24h trend has one slot per hour, so
// the trailing run of non-up hours bounds the outage from below: "≥ 3h", or "> 24h".
function downHours(m) {
  const t = m.trend_24h || []
  let n = 0
  for (let i = t.length - 1; i >= 0 && t[i] !== true; i--) n++
  return n
}
const svc = computed(() => {
  let up = 0, down = 0, pending = 0, longest = 0
  for (const m of wsMonitors.value) {
    if (m.up === true) up++
    else if (m.up === false) { down++; longest = Math.max(longest, downHours(m)) }
    else pending++
  }
  const longestText = !down ? '' : longest >= 24 ? '> 24h' : longest > 0 ? `≥ ${longest}h` : '< 1h'
  return { total: wsMonitors.value.length, up, down, pending, longest: longestText }
})
const firing = computed(() => alerts.value.filter((a) => a.enabled && a.firing === true).length)

// average service uptime (SLA) over services that have recent checks
const upPct = (m) => (m.recent && m.recent.length ? Math.round((m.recent.filter(Boolean).length / m.recent.length) * 100) : null)
const svcUptime = computed(() => {
  const ups = wsMonitors.value.map(upPct).filter((u) => u != null)
  return ups.length ? Math.round(ups.reduce((a, b) => a + b, 0) / ups.length) : null
})

// ---- tiles ----
// Action tiles: large, only tinted when the number is non-zero.
const actions = computed(() => [
  { label: 'Hosts down', value: host.value.down, sub: host.value.down ? `longest ${host.value.longest}` : 'all reporting', icon: 'wifi-off', to: { name: 'attention', query: { ...nsq.value, status: 'down' } }, bad: host.value.down > 0, color: 'down' },
  { label: 'Critical', value: host.value.crit, sub: 'over critical threshold', icon: 'alert-triangle', to: { name: 'attention', query: { ...nsq.value, status: 'crit' } }, bad: host.value.crit > 0, color: 'crit' },
  { label: 'Warning', value: host.value.warn, sub: 'over warning threshold', icon: 'alert-triangle', to: { name: 'attention', query: { ...nsq.value, status: 'warn' } }, bad: host.value.warn > 0, color: 'warn' },
  { label: 'Services down', value: svc.value.down, sub: svc.value.down ? `longest ${svc.value.longest}` : svc.value.total ? 'all up' : 'none configured', icon: 'wifi-off', to: { name: 'monitors', query: { ...nsq.value, status: 'down' } }, bad: svc.value.down > 0, color: 'down' },
  { label: 'Alerts firing', value: firing.value, sub: firing.value ? 'rules in breach' : 'quiet', icon: 'bell', to: { name: 'alerts', query: nsq.value }, bad: firing.value > 0, color: 'down' },
])
// Info tiles: compact, never tinted.
const info = computed(() => [
  { label: 'Hosts', value: host.value.total, sub: `${host.value.up} up`, icon: 'server', to: { name: 'systems', query: nsq.value } },
  ...(clusterAgg.value.total > 0
    ? [
        { label: 'Clusters', value: clusterAgg.value.total, sub: `${clusterAgg.value.online} online`, icon: 'server', to: { name: 'clusters', query: nsq.value } },
        { label: 'Nodes', value: clusterAgg.value.nodes || '—', icon: 'fleet', to: { name: 'clusters', query: nsq.value } },
        { label: 'Pods running', value: clusterAgg.value.pods || '—', icon: 'pulse', to: { name: 'clusters', query: nsq.value } },
        { label: 'CPU used', value: clusterAgg.value.cpu ? `${Math.round(clusterAgg.value.cpu / 1000)} cores` : '—', icon: 'cpu', to: { name: 'clusters', query: nsq.value } },
      ]
    : []),
  { label: 'Services', value: svc.value.total, sub: `${svc.value.up} up`, icon: 'service', to: { name: 'monitors', query: nsq.value } },
  { label: 'Avg uptime', value: svcUptime.value == null ? 'N/A' : `${svcUptime.value}%`, sub: 'recent checks', icon: 'uptime', to: { name: 'monitors', query: nsq.value } },
])

const BAD_BORDER = { down: 'border-down/40 bg-down/10', crit: 'border-crit/40 bg-crit/10', warn: 'border-warn/40 bg-warn/10' }
const BAD_TEXT = { down: 'text-down', crit: 'text-crit', warn: 'text-warn' }

const { loaded, reload: load } = useCached({
  key: () => 'overview:' + selectedWs.value.join(','),
  load: async () => {
    const nss = workspaces.value
    const [sys, mons, thr, alertLists] = await Promise.all([
      api.get('/api/systems').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/thresholds').catch(() => ({})),
      Promise.all(nss.map((n) => api.get(`/api/workspaces/${n.id}/alerts`).catch(() => []))),
    ])
    // Cluster roll-up: one batch call for all clusters (not one per cluster).
    const csums = {}
    if (sys.some((s) => s.kind === 'k8s-cluster')) {
      const arr = await api.get('/api/kube/summaries').catch(() => [])
      for (const s of arr) csums[s.system_id] = s
    }
    return { sys, mons, thr, alerts: alertLists.flat(), csums }
  },
  apply: (d) => {
    systems.value = d.sys; monitors.value = d.mons
    thresholds.value = d.thr || {}; alerts.value = d.alerts
    clusterSums.value = d.csums || {}
  },
})

watch(() => route.query.ws, load)
onMounted(async () => {
  try { workspaces.value = await api.get('/api/workspaces') } catch {}
  await load()
  // 30s — one `load()` is several API calls (incl. the cluster roll-up); 10s was
  // hammering Postgres for data that changes far more slowly.
  timer = setInterval(load, 30000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Overview">
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-6">
      <section>
        <h2 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">Needs attention</h2>
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
          <RouterLink v-for="t in actions" :key="t.label" :to="t.to"
            class="flex min-h-[104px] flex-col rounded-xl border p-4 transition hover:border-accent/60"
            :class="t.bad ? BAD_BORDER[t.color] : 'border-line bg-surface'">
            <div class="flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-faint">
              <VIcon :name="t.icon" :size="13" class="shrink-0" />{{ t.label }}
            </div>
            <div class="mt-auto font-mono text-metric font-extrabold tabular-nums" :class="t.bad ? BAD_TEXT[t.color] : 'text-ok'">{{ t.value }}</div>
            <div v-if="t.sub" class="mt-0.5 text-xs" :class="t.bad ? BAD_TEXT[t.color] : 'text-faint'">{{ t.sub }}</div>
          </RouterLink>
        </div>
      </section>
      <section>
        <h2 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">Inventory</h2>
        <div class="flex flex-wrap gap-2">
          <RouterLink v-for="t in info" :key="t.label" :to="t.to"
            class="flex min-w-[140px] flex-1 items-center gap-3 rounded-lg border border-line bg-surface px-3 py-2 transition hover:border-accent/60">
            <VIcon :name="t.icon" :size="14" class="shrink-0 text-faint" />
            <div class="min-w-0">
              <div class="text-[10px] uppercase tracking-wider text-faint">{{ t.label }}</div>
              <div class="font-mono text-sm font-semibold tabular-nums text-fg">{{ t.value }}<span v-if="t.sub" class="ml-1.5 text-xs font-normal text-faint">{{ t.sub }}</span></div>
            </div>
          </RouterLink>
        </div>
      </section>
    </div>
  </AppShell>
</template>
