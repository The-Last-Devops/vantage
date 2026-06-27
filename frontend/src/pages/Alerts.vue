<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { useCached } from '../lib/cache'

const route = useRoute()
const router = useRouter()

const namespaces = ref([])
const selectedNsNames = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const activeNs = computed(() =>
  selectedNsNames.value.length
    ? namespaces.value.filter((n) => selectedNsNames.value.includes(n.name))
    : namespaces.value,
)
const alerts = ref([])
const channels = ref([])
const types = ref([])
let timer = null

// ---- channel badge helpers (kind → color/icon) ----
const ICONS = {
  telegram: { fill: true, body: '<path d="M21.9 4.3 18.6 20c-.2 1-.9 1.3-1.8.8l-4.9-3.6-2.4 2.3c-.3.3-.5.5-1 .5l.3-5 9.1-8.2c.4-.4-.1-.6-.6-.2L6.2 13.1l-4.8-1.5c-1-.3-1-1 .2-1.5l18.7-7.2c.9-.3 1.6.2 1.3 1.4z"/>' },
  slack: { fill: true, body: '<path d="M5 15a2 2 0 1 1-2-2h2v2zm1 0a2 2 0 0 1 4 0v5a2 2 0 0 1-4 0v-5zM9 5a2 2 0 1 1 2-2v2H9zm0 1a2 2 0 0 1 0 4H4a2 2 0 0 1 0-4h5zm10 4a2 2 0 1 1 2 2h-2v-2zm-1 0a2 2 0 0 1-4 0V5a2 2 0 0 1 4 0v5zm-3 9a2 2 0 1 1-2 2v-2h2zm0-1a2 2 0 0 1 0-4h5a2 2 0 0 1 0 4h-5z"/>' },
  discord: { fill: true, body: '<path d="M20 4.4A19 19 0 0 0 15.3 3l-.2.5c1.7.4 2.5.9 3.4 1.5a13 13 0 0 0-9.9 0c.9-.6 1.8-1.1 3.4-1.5L11.7 3A19 19 0 0 0 7 4.4C4 8.9 3.2 13.3 3.6 17.6a19 19 0 0 0 5.7 2.9l.5-.8c-.8-.3-1.5-.7-2.2-1.1l.5-.4a13.6 13.6 0 0 0 11.6 0l.5.4c-.7.4-1.4.8-2.2 1.1l.5.8a19 19 0 0 0 5.7-2.9c.5-5-.8-9.3-3.6-13.2zM9.3 14.9c-1 0-1.9-1-1.9-2.1s.8-2.1 1.9-2.1 1.9 1 1.9 2.1-.9 2.1-1.9 2.1zm5.4 0c-1 0-1.9-1-1.9-2.1s.8-2.1 1.9-2.1 1.9 1 1.9 2.1-.9 2.1-1.9 2.1z"/>' },
  webhook: { fill: false, body: '<path d="M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 0 1 2 17a4 4 0 0 1 4-4"/><path d="m6 17 3.13-5.78c.53-.97.43-2.22-.21-3.08A4 4 0 1 1 16 4"/><path d="m12 6 3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 0 1 0 8"/>' },
  email: { fill: false, body: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m2 7 10 6 10-6"/>' },
  sms: { fill: false, body: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>' },
  push: { fill: false, body: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>' },
  incident: { fill: false, body: '<path d="m10.29 3.86-8.48 14.7A2 2 0 0 0 3.53 21h16.94a2 2 0 0 0 1.72-2.44L13.71 3.86a2 2 0 0 0-3.42 0Z"/><path d="M12 9v4M12 17h.01"/>' },
  chat: { fill: false, body: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>' },
}
const typeByKind = (k) => types.value.find((t) => t.kind === k)
function iconSvg(name, size = 14) {
  const ic = ICONS[name] || ICONS.chat
  const attrs = ic.fill ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" ${attrs}>${ic.body}</svg>`
}
const chanColor = (kind) => typeByKind(kind)?.color || 'rgb(var(--surface2))'
const chanFg = (kind) => typeByKind(kind)?.fg || 'rgb(var(--fg))'
const chanIcon = (kind) => typeByKind(kind)?.icon || 'chat'

// ---- derived row fields ----
const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
const isSvc = (a) => a.target_kind === 'monitor' || a.target_kind === 'all_services'
function condText(a) {
  const c = a.condition || {}
  if (isSvc(a)) return 'is DOWN'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return '—'
}
function stateOf(a) {
  if (!a.enabled) return 'disabled'
  if (a.firing === true) return 'firing'
  if (a.firing === false) return 'ok'
  return 'pending'
}
const TONE = { firing: 'down', ok: 'ok', pending: 'warn', disabled: 'muted' }
const STATE_LABEL = { firing: 'Firing', ok: 'OK', pending: 'Pending', disabled: 'Disabled' }
const renotifyText = (a) => (a.renotify_secs ? `every ${Math.round(a.renotify_secs / 60)}m` : 'once')

const tableRows = computed(() =>
  alerts.value.map((a) => ({ ...a, state: STATE_LABEL[stateOf(a)], cond: condText(a) })),
)
const firingCount = computed(() => alerts.value.filter((a) => stateOf(a) === 'firing').length)
const columns = [
  { key: 'state', label: 'State', sortable: true, width: '116px' },
  { key: 'target_name', label: 'Source', sortable: true, nowrap: false },
  { key: 'cond', label: 'Condition', sortable: true },
  { key: 'channels', label: 'Channels' },
  { key: 'namespace', label: 'Namespace', sortable: true },
  { key: 'renotify_secs', label: 'Re-notify', sortable: true },
  { key: 'enabled', label: 'On', align: 'center', width: '70px' },
  { key: 'actions', label: '', align: 'right', width: '116px' },
]

const { loaded, reload: load } = useCached({
  key: () => 'alerts:' + activeNs.value.map((n) => n.id).join(','),
  load: async () => {
    const nss = activeNs.value
    if (!nss.length) return { alerts: [], channels: [] }
    const [aLists, allChannels] = await Promise.all([
      Promise.all(nss.map((n) =>
        api.get(`/api/namespaces/${n.id}/alerts`)
          .then((rows) => rows.map((r) => ({ ...r, namespace: n.name })))
          .catch(() => []),
      )),
      api.get('/api/channels').catch(() => []),
    ])
    const seen = new Set()
    const merged = aLists
      .flat()
      .filter((a) => !seen.has(a.id) && seen.add(a.id))
      .sort((a, b) =>
        (a.namespace || '').localeCompare(b.namespace || '') ||
        String(a.target_name).localeCompare(String(b.target_name)) ||
        String(a.id).localeCompare(String(b.id)),
      )
    return { alerts: merged, channels: allChannels }
  },
  apply: (d) => { alerts.value = d.alerts; channels.value = d.channels },
})
watch(() => route.query.ns, load)

// ---- row actions ----
async function toggle(row) {
  // `row` is a copy from tableRows — flip the SOURCE item so the table re-renders.
  const a = alerts.value.find((x) => x.id === row.id)
  if (!a) return
  const prev = a.enabled
  a.enabled = !a.enabled
  try { await api.patch(`/api/alerts/${a.id}`, { enabled: a.enabled }) }
  catch (e) { a.enabled = prev; alert(`Failed (${e.status}).`) }
}
async function removeAlert(a) {
  if (!(await confirm({ title: 'Delete alert rule?', message: 'This rule will stop watching and notifying. This cannot be undone.', danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/alerts/${a.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}
const testState = ref({})
async function testAlert(a) {
  testState.value = { ...testState.value, [a.id]: 'testing' }
  try { await api.post(`/api/alerts/${a.id}/test`); testState.value = { ...testState.value, [a.id]: 'ok' } }
  catch { testState.value = { ...testState.value, [a.id]: 'fail' } }
  setTimeout(() => { testState.value = { ...testState.value, [a.id]: undefined } }, 3000)
}

// ---- bulk actions ----
const selectedIds = ref([])
async function bulkEnable(rows, val) {
  await Promise.all(rows.map((a) => api.patch(`/api/alerts/${a.id}`, { enabled: val }).catch(() => {})))
  selectedIds.value = []
  await load()
}
async function bulkDelete(rows) {
  if (!(await confirm({ title: `Delete ${rows.length} alert rule(s)?`, message: 'They will stop notifying. This cannot be undone.', danger: true, confirmText: `Delete ${rows.length}` }))) return
  await Promise.all(rows.map((a) => api.del(`/api/alerts/${a.id}`).catch(() => {})))
  selectedIds.value = []
  await load()
}

// ---- navigation ----
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const openNew = () => router.push({ name: 'alert-new', query: nsq.value })
const openEdit = (a) => router.push({ name: 'alert-edit', params: { id: a.id }, query: nsq.value })

onMounted(async () => {
  try { types.value = await api.get('/api/channel-types') } catch {}
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  await load()
  timer = setInterval(load, 15000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Alert rules">
    <template #title-after><span class="text-sm text-faint">{{ alerts.length }} rules<span v-if="firingCount" class="text-rose-500"> · {{ firingCount }} firing</span></span></template>
    <template #actions>
      <button @click="openNew" class="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-sm font-semibold text-accentfg hover:opacity-90">
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> New rule
      </button>
    </template>
    <div class="space-y-4">
      <PageLoader v-if="!loaded" />
      <DataTable v-else v-model:selected="selectedIds" :columns="columns" :rows="tableRows" :row-key="(r) => r.id"
        selectable clickable @row-click="openEdit"
        :filter-keys="['target_name', 'namespace', 'cond', 'state']" filter-placeholder="Filter rules…"
        empty="No alert rules yet. Click New rule to wire one up.">
        <template #bulk="{ selected, disabled }">
          <button :disabled="disabled" @click="bulkEnable(selected, true)" class="rounded-lg border border-line bg-surface2 px-2.5 py-1.5 text-xs font-medium text-fg hover:border-accent/50 disabled:cursor-not-allowed disabled:opacity-40">Enable</button>
          <button :disabled="disabled" @click="bulkEnable(selected, false)" class="rounded-lg border border-line bg-surface2 px-2.5 py-1.5 text-xs font-medium text-fg hover:border-accent/50 disabled:cursor-not-allowed disabled:opacity-40">Disable</button>
          <button :disabled="disabled" @click="bulkDelete(selected)" class="rounded-lg border border-rose-500/35 px-2.5 py-1.5 text-xs font-medium text-rose-500 hover:bg-rose-500/10 disabled:cursor-not-allowed disabled:opacity-40">Delete</button>
        </template>

        <template #cell-state="{ row }">
          <StatePill :tone="TONE[stateOf(row)]" :label="row.state" />
        </template>

        <template #cell-target_name="{ row }">
          <div class="flex items-center gap-2">
            <svg v-if="isSvc(row)" class="h-[15px] w-[15px] shrink-0 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
            <svg v-else class="h-[15px] w-[15px] shrink-0 text-sky-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>
            <span class="font-medium text-fg">{{ row.target_name || '—' }}</span>
            <span class="text-xs text-faint">{{ isSvc(row) ? 'Service' : 'Host' }}</span>
          </div>
        </template>

        <template #cell-cond="{ row }">
          <span class="font-mono text-[13px] text-[rgb(var(--warn))]">{{ row.cond }}</span>
        </template>

        <template #cell-channels="{ row }">
          <span class="flex flex-wrap items-center gap-1">
            <span v-for="ch in row.channels" :key="ch.id" v-tip="ch.name" class="grid h-[22px] w-[22px] place-items-center rounded-md" :style="{ background: chanColor(ch.kind), color: chanFg(ch.kind) }" v-html="iconSvg(chanIcon(ch.kind), 13)"></span>
            <span v-if="!row.channels.length" class="text-xs text-rose-500">none</span>
          </span>
        </template>

        <template #cell-namespace="{ row }">
          <span class="text-muted">{{ row.namespace }}</span>
        </template>

        <template #cell-renotify_secs="{ row }">
          <span class="text-muted" v-tip="row.renotify_secs ? 'Re-notifies while still firing' : 'Notifies once per incident'">{{ renotifyText(row) }}</span>
        </template>

        <template #cell-enabled="{ row }">
          <button @click.stop="toggle(row)" v-tip="row.enabled ? 'Disable' : 'Enable'" class="relative inline-block h-[22px] w-10 shrink-0 rounded-full align-middle transition-colors" :class="row.enabled ? 'bg-accent' : 'bg-line'">
            <span class="absolute top-0.5 h-[18px] w-[18px] rounded-full transition-all" :class="row.enabled ? 'left-[20px] bg-accentfg' : 'left-0.5 bg-fg'"></span>
          </button>
        </template>

        <template #cell-actions="{ row }">
          <div class="flex items-center justify-end gap-1">
            <span v-if="testState[row.id] === 'ok'" class="text-xs text-accent">✓</span>
            <span v-else-if="testState[row.id] === 'fail'" class="text-xs text-rose-500">✗</span>
            <button @click.stop="testAlert(row)" :disabled="testState[row.id] === 'testing'" class="rounded-lg border border-line bg-surface2 px-2 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50" v-tip="`Send test`">Test</button>
            <button @click.stop="openEdit(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="`Edit`"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg></button>
            <button @click.stop="removeAlert(row)" class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-rose-500" v-tip="`Delete`"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg></button>
          </div>
        </template>
      </DataTable>
    </div>
  </AppShell>
</template>
