<script setup>
// Overview / Dashboard: a single uniform grid of status tiles (no scattered lists,
// charts or event feeds). Every tile is the same size, shows one number, highlights
// only when something needs attention, and links to the page it summarises. Sections
// group the tiles; aggregates across the selected namespaces (?ns=).
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { useAuth } from '../stores/auth'
import { online, hostState, DEFAULT_THR } from '../lib/triage'

const route = useRoute()
const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const inNs = (s) => selectedNs.value.length === 0 || selectedNs.value.includes(s.namespace)

const systems = ref([])
const monitors = ref([])
const events = ref([])
const thresholds = ref({})
const namespaces = ref([])
const alerts = ref([])
const backup = ref(null) // { enabled, last_backup_at } (admin only)
const tfa = ref({ enabled: false })
const passkeys = ref([])
const dataStats = ref(null) // { db_size, ... } (admin only)
const memberCount = ref(null) // user count (admin only)
let timer = null

const thrOf = (s) => thresholds.value[s.namespace] || DEFAULT_THR
const hosts = computed(() => systems.value.filter(inNs))
const nsMonitors = computed(() => monitors.value.filter(inNs).filter((m) => m.enabled))

// ---- counts ----
const host = computed(() => {
  let up = 0, down = 0, crit = 0, warn = 0
  for (const s of hosts.value) {
    if (online(s)) up++
    const st = hostState(s, thrOf(s))
    if (st === 'down') down++
    else if (st === 'crit') crit++
    else if (st === 'warn') warn++
  }
  return { total: hosts.value.length, up, down, crit, warn }
})
const svc = computed(() => {
  let up = 0, down = 0, pending = 0
  for (const m of nsMonitors.value) {
    if (m.up === true) up++
    else if (m.up === false) down++
    else pending++
  }
  return { total: nsMonitors.value.length, up, down, pending }
})
const firing = computed(() => alerts.value.filter((a) => a.enabled && a.firing === true).length)
const events24 = computed(() => events.value.length)

// average service uptime (SLA) over services that have recent checks
const upPct = (m) => (m.recent && m.recent.length ? Math.round((m.recent.filter(Boolean).length / m.recent.length) * 100) : null)
const svcUptime = computed(() => {
  const ups = nsMonitors.value.map(upPct).filter((u) => u != null)
  return ups.length ? Math.round(ups.reduce((a, b) => a + b, 0) / ups.length) : null
})

// agents on an older version than the newest one reporting
function cmpVer(a, b) {
  const pa = String(a).split('.').map(Number), pb = String(b).split('.').map(Number)
  for (let i = 0; i < 3; i++) if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0)
  return 0
}
const agentOutdated = computed(() => {
  const vers = hosts.value.map((s) => s.agent_version).filter(Boolean)
  if (!vers.length) return 0
  const latest = vers.slice().sort(cmpVer).pop()
  return hosts.value.filter((s) => s.agent_version && cmpVer(s.agent_version, latest) < 0).length
})

const secured = computed(() => tfa.value.enabled || passkeys.value.length > 0)
const securedSub = computed(() => {
  const bits = []
  if (tfa.value.enabled) bits.push('Authenticator')
  if (passkeys.value.length) bits.push(`${passkeys.value.length} passkey${passkeys.value.length > 1 ? 's' : ''}`)
  return bits.join(' · ') || 'Not enabled'
})
const backupTile = computed(() => {
  if (!backup.value) return { value: '—', sub: 'unknown', bad: false }
  if (!backup.value.enabled) return { value: 'Off', sub: 'no schedule', bad: true, color: 'down' }
  const last = backup.value.last_backup_at ? new Date(backup.value.last_backup_at) : null
  if (!last) return { value: 'Pending', sub: 'never run', bad: true, color: 'warn' }
  const days = (Date.now() - last.getTime()) / 86400000
  return { value: 'On', sub: `last ${fmtAgo(last)}`, bad: days > 2, color: 'warn', good: days <= 2 }
})
function fmtAgo(d) {
  const s = Math.max(0, (Date.now() - d.getTime()) / 1000)
  if (s < 3600) return `${Math.round(s / 60)}m ago`
  if (s < 86400) return `${Math.round(s / 3600)}h ago`
  return `${Math.round(s / 86400)}d ago`
}

// ---- tiles (uniform; bad=highlight) ----
const sections = computed(() => [
  {
    title: 'Hosts',
    items: [
      { label: 'Hosts', value: host.value.total, sub: `${host.value.up} up`, icon: 'server', to: { name: 'systems', query: nsq.value }, good: host.value.total > 0 && host.value.down === 0 && host.value.crit === 0 && host.value.warn === 0 },
      { label: 'Down', value: host.value.down, icon: 'wifi-off', to: { name: 'attention', query: nsq.value }, bad: host.value.down > 0, color: 'down' },
      { label: 'Critical', value: host.value.crit, icon: 'alert-triangle', to: { name: 'attention', query: nsq.value }, bad: host.value.crit > 0, color: 'crit' },
      { label: 'Warning', value: host.value.warn, icon: 'alert-triangle', to: { name: 'attention', query: nsq.value }, bad: host.value.warn > 0, color: 'warn' },
    ],
  },
  {
    title: 'Services',
    items: [
      { label: 'Services', value: svc.value.total, sub: `${svc.value.up} up`, icon: 'service', to: { name: 'monitors', query: nsq.value }, good: svc.value.total > 0 && svc.value.down === 0 },
      { label: 'Down', value: svc.value.down, icon: 'wifi-off', to: { name: 'monitors', query: { ...nsq.value, status: 'down' } }, bad: svc.value.down > 0, color: 'down' },
      { label: 'Avg uptime', value: svcUptime.value == null ? 'N/A' : `${svcUptime.value}%`, sub: 'recent checks', icon: 'uptime', to: { name: 'monitors', query: nsq.value }, bad: svcUptime.value != null && svcUptime.value < 99, color: svcUptime.value != null && svcUptime.value < 90 ? 'down' : 'warn', good: svcUptime.value != null && svcUptime.value >= 99.9 },
    ],
  },
  {
    title: 'Operations',
    items: [
      { label: 'Alerts firing', value: firing.value, icon: 'bell', to: { name: 'alerts', query: nsq.value }, bad: firing.value > 0, color: 'down' },
      { label: 'Events · 24h', value: events24.value, icon: 'pulse', to: { name: 'events', query: nsq.value } },
      { label: 'Agent updates', value: agentOutdated.value, icon: 'deploy', to: { name: 'systems', query: nsq.value }, bad: agentOutdated.value > 0, color: 'warn' },
      ...(isAdmin.value
        ? [{ label: 'Backup', value: backupTile.value.value, sub: backupTile.value.sub, icon: 'refresh', to: { name: 'backup' }, bad: backupTile.value.bad, color: backupTile.value.color, good: backupTile.value.good }]
        : []),
    ],
  },
  {
    // Security + system folded into one grid so the rows fill out (Two-factor alone
    // left a near-empty row). Admin sees 4 tiles (a full row); non-admin sees just
    // the one Two-factor tile, left-aligned.
    title: 'Account & system',
    items: [
      { label: 'Two-factor', value: secured.value ? 'On' : 'Off', sub: securedSub.value, icon: 'shield', to: { name: 'security' }, bad: !secured.value, color: 'warn', good: secured.value },
      ...(isAdmin.value
        ? [
            { label: 'Database', value: dataStats.value?.db_size || '—', sub: 'data DB size', icon: 'disk', to: { name: 'data' } },
            { label: 'Namespaces', value: namespaces.value.length, icon: 'globe', to: { name: 'namespaces' } },
            { label: 'Members', value: memberCount.value ?? '—', icon: 'user', to: { name: 'members' } },
          ]
        : []),
    ],
  },
])

const BAD_BORDER = { down: 'border-down/40 bg-down/10', crit: 'border-crit/40 bg-crit/10', warn: 'border-warn/40 bg-warn/10' }
const BAD_TEXT = { down: 'text-down', crit: 'text-crit', warn: 'text-warn' }

const { loaded, reload: load } = useCached({
  key: () => 'overview:' + selectedNs.value.join(','),
  load: async () => {
    const nss = namespaces.value
    const admin = isAdmin.value
    const [sys, mons, evs, thr, alertLists, bk, t2, pks, ds, users] = await Promise.all([
      api.get('/api/systems').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/events?range=24h').catch(() => []),
      api.get('/api/thresholds').catch(() => ({})),
      Promise.all(nss.map((n) => api.get(`/api/namespaces/${n.id}/alerts`).catch(() => []))),
      admin ? api.get('/api/admin/backup/schedule').catch(() => null) : Promise.resolve(null),
      api.get('/api/me/2fa').catch(() => ({ enabled: false })),
      api.get('/api/me/passkeys').catch(() => []),
      admin ? api.get('/api/admin/data').catch(() => null) : Promise.resolve(null),
      admin ? api.get('/api/users').catch(() => []) : Promise.resolve([]),
    ])
    return { sys, mons, evs, thr, alerts: alertLists.flat(), bk, t2, pks, ds, users }
  },
  apply: (d) => {
    systems.value = d.sys; monitors.value = d.mons; events.value = d.evs
    thresholds.value = d.thr || {}; alerts.value = d.alerts
    backup.value = d.bk; tfa.value = d.t2 || { enabled: false }; passkeys.value = d.pks || []
    dataStats.value = d.ds; memberCount.value = Array.isArray(d.users) ? d.users.length : null
  },
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
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-6">
      <section v-for="sec in sections" :key="sec.title">
        <h2 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">{{ sec.title }}</h2>
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
          <RouterLink v-for="t in sec.items" :key="t.label" :to="t.to"
            class="flex min-h-[104px] flex-col rounded-xl border p-4 transition hover:border-accent/60"
            :class="t.bad ? BAD_BORDER[t.color] : t.good ? 'border-ok/40 bg-ok/10' : 'border-line bg-surface'">
            <div class="flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-faint">
              <VIcon :name="t.icon" :size="13" class="shrink-0" />{{ t.label }}
            </div>
            <div class="mt-auto font-mono text-metric font-extrabold tabular-nums" :class="t.bad ? BAD_TEXT[t.color] : t.good ? 'text-ok' : 'text-fg'">{{ t.value }}</div>
            <div v-if="t.sub" class="mt-0.5 text-xs text-faint">{{ t.sub }}</div>
          </RouterLink>
        </div>
      </section>
    </div>
  </AppShell>
</template>
