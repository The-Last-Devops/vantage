<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'

const route = useRoute()
const selectedNsName = () => {
  const sel = (route.query.ns || '').split(',').filter(Boolean)
  return sel.length === 1 ? sel[0] : null
}

const namespaces = ref([])
const nsId = ref('')
const nsName = computed(() => namespaces.value.find((n) => n.id === nsId.value)?.name || '')
const alerts = ref([])
const events = ref([])
const channels = ref([])
const monitors = ref([])
const systems = ref([])
const err = ref('')
const showForm = ref(false)
let timer = null

const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function condText(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor') return 'fires when the monitor is DOWN'
  if (c.offline_secs) return `fires when offline > ${c.offline_secs}s`
  if (c.metric) return `fires when ${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return '—'
}
function stateOf(a) {
  if (!a.enabled) return 'disabled'
  if (a.firing === true) return 'firing'
  if (a.firing === false) return 'ok'
  return 'pending'
}
const STATE = {
  firing: { label: 'Firing', dot: 'bg-red-500', text: 'text-red-500' },
  ok: { label: 'OK', dot: 'bg-accent', text: 'text-accent' },
  disabled: { label: 'Disabled', dot: 'bg-faint', text: 'text-faint' },
  pending: { label: 'Pending', dot: 'bg-amber-400', text: 'text-amber-400' },
}
const chIcon = (k) => ({ telegram: '✈️', slack: '💬', discord: '🎮', webhook: '🔗' }[k] || '🔔')
function ago(iso) {
  if (!iso) return ''
  let s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000)
  const d = Math.floor(s / 86400); s -= d * 86400
  const h = Math.floor(s / 3600); s -= h * 3600
  const m = Math.floor(s / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  if (m) return `${m}m`
  return `${Math.floor(s)}s`
}
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

async function loadAlerts() {
  if (!nsId.value) { alerts.value = []; events.value = []; return }
  try {
    alerts.value = await api.get(`/api/namespaces/${nsId.value}/alerts`)
    channels.value = await api.get(`/api/namespaces/${nsId.value}/channels`)
    events.value = await api.get(`/api/namespaces/${nsId.value}/alert-events`)
  } catch { alerts.value = [] }
}
watch(nsId, loadAlerts)

// create form
const na = ref({ channel_id: '', targetType: 'monitor', monitor_id: '', system_id: '', condType: 'metric', metric: 'cpu_percent', op: '>', value: 90, offline_secs: 120, cooldown: 300 })
const sysInNs = computed(() => systems.value.filter((s) => s.namespace === nsName.value))

async function addAlert() {
  err.value = ''
  if (!na.value.channel_id) { err.value = 'Pick a channel.'; return }
  const body = { channel_id: na.value.channel_id, cooldown_secs: Number(na.value.cooldown) || 300 }
  if (na.value.targetType === 'monitor') {
    if (!na.value.monitor_id) { err.value = 'Pick a monitor.'; return }
    body.monitor_id = na.value.monitor_id
  } else {
    if (!na.value.system_id) { err.value = 'Pick a host.'; return }
    body.system_id = na.value.system_id
    body.condition = na.value.condType === 'offline'
      ? { offline_secs: Number(na.value.offline_secs) || 120 }
      : { metric: na.value.metric, op: na.value.op, value: Number(na.value.value) }
  }
  try { await api.post(`/api/namespaces/${nsId.value}/alerts`, body); showForm.value = false; await loadAlerts() }
  catch (e) { err.value = e.status === 403 ? 'You need editor access.' : `Failed (${e.status}).` }
}
async function removeAlert(a) {
  if (!confirm('Delete this alert rule?')) return
  try { await api.del(`/api/alerts/${a.id}`); await loadAlerts() } catch (e) { alert(`Failed (${e.status}).`) }
}
async function toggle(a) {
  try { await api.patch(`/api/alerts/${a.id}`, { enabled: !a.enabled }); await loadAlerts() } catch (e) { alert(`Failed (${e.status}).`) }
}
const testState = ref({})
async function testAlert(a) {
  testState.value = { ...testState.value, [a.id]: 'testing' }
  try { await api.post(`/api/alerts/${a.id}/test`); testState.value = { ...testState.value, [a.id]: 'ok' } }
  catch { testState.value = { ...testState.value, [a.id]: 'fail' } }
  setTimeout(() => { testState.value = { ...testState.value, [a.id]: undefined } }, 3000)
}
// inline channel change
async function setChannel(a, channel_id) {
  try { await api.patch(`/api/alerts/${a.id}`, { channel_id }); await loadAlerts() } catch (e) { alert(`Failed (${e.status}).`) }
}

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  try { monitors.value = await api.get('/api/monitors') } catch {}
  try { systems.value = await api.get('/api/systems') } catch {}
  const match = namespaces.value.find((n) => n.name === selectedNsName())
  nsId.value = (match || namespaces.value[0])?.id || ''
  timer = setInterval(loadAlerts, 15000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Alert rules">
    <div class="max-w-4xl space-y-5">
      <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold text-fg">Rules</h2>
        <select v-model="nsId" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
          <option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option>
        </select>
        <button @click="showForm = !showForm" class="ml-auto flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> New rule
        </button>
      </div>
      <p class="text-xs text-faint">Fire a channel when a monitor goes down or a host breaches a threshold. Manage channels under <b>Alert › Notify channel</b>.</p>

      <!-- create -->
      <form v-if="showForm" @submit.prevent="addAlert" class="space-y-2 rounded-xl border border-line bg-surface p-4">
        <div class="flex flex-wrap items-end gap-3">
          <label class="text-xs text-faint">Channel<select v-model="na.channel_id" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="">—</option><option v-for="c in channels" :key="c.id" :value="c.id">{{ c.name }}</option></select></label>
          <label class="text-xs text-faint">Target<select v-model="na.targetType" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="monitor">Monitor (down)</option><option value="system">Host (condition)</option></select></label>
          <label v-if="na.targetType === 'monitor'" class="text-xs text-faint">Monitor<select v-model="na.monitor_id" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="">—</option><option v-for="m in monitors" :key="m.id" :value="m.id">{{ m.name }}</option></select></label>
          <label v-else class="text-xs text-faint">Host<select v-model="na.system_id" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="">—</option><option v-for="s in sysInNs" :key="s.id" :value="s.id">{{ s.name }}</option></select></label>
        </div>
        <div v-if="na.targetType === 'system'" class="flex flex-wrap items-end gap-3">
          <label class="text-xs text-faint">Condition<select v-model="na.condType" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="metric">Metric threshold</option><option value="offline">Offline</option></select></label>
          <template v-if="na.condType === 'metric'">
            <label class="text-xs text-faint">Metric<select v-model="na.metric" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="cpu_percent">CPU %</option><option value="mem_percent">Memory %</option><option value="load1">Load 1m</option></select></label>
            <label class="text-xs text-faint">Op<select v-model="na.op" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option>&gt;</option><option>&lt;</option><option>&gt;=</option><option>&lt;=</option></select></label>
            <label class="text-xs text-faint">Value<input v-model.number="na.value" type="number" class="mt-1 block w-24 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          </template>
          <label v-else class="text-xs text-faint">Offline after (s)<input v-model.number="na.offline_secs" type="number" class="mt-1 block w-28 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
        </div>
        <div class="flex items-center gap-3">
          <label class="text-xs text-faint">Cooldown (s)<input v-model.number="na.cooldown" type="number" class="mt-1 block w-28 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <button type="submit" class="mt-4 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">Add rule</button>
          <span v-if="err" class="mt-4 text-xs text-rose-400">{{ err }}</span>
        </div>
      </form>

      <!-- rule cards -->
      <p v-if="!alerts.length" class="rounded-xl border border-line bg-surface p-6 text-center text-sm text-muted">No alert rules in this namespace yet.</p>
      <div v-else class="space-y-2">
        <div v-for="a in alerts" :key="a.id" class="rounded-xl border border-line bg-surface p-3" :class="stateOf(a) === 'firing' ? 'border-red-500/40' : ''">
          <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
            <span class="inline-flex items-center gap-1.5 text-sm font-semibold" :class="STATE[stateOf(a)].text">
              <span class="h-2 w-2 rounded-full" :class="STATE[stateOf(a)].dot"></span>{{ STATE[stateOf(a)].label }}
            </span>
            <div class="min-w-0">
              <div class="truncate text-sm font-medium text-fg">
                <span class="rounded bg-surface2 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-faint">{{ a.target_kind }}</span>
                {{ a.target_name || '—' }}
              </div>
              <div class="text-xs text-muted">{{ condText(a) }}</div>
            </div>
            <span v-if="a.since && stateOf(a) !== 'disabled'" class="text-xs text-faint">{{ stateOf(a) === 'firing' ? 'firing' : 'ok' }} for {{ ago(a.since) }}</span>
            <div class="ml-auto flex items-center gap-2">
              <span class="text-xs text-muted" :title="'channel'">{{ chIcon(a.channel_kind) }} {{ a.channel_name }}</span>
              <span v-if="testState[a.id] === 'ok'" class="text-xs text-accent">✓ sent</span>
              <span v-else-if="testState[a.id] === 'fail'" class="text-xs text-rose-400">✗ failed</span>
              <button @click="testAlert(a)" :disabled="testState[a.id] === 'testing'" class="rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50">{{ testState[a.id] === 'testing' ? 'Testing…' : 'Test' }}</button>
              <button @click="toggle(a)" class="rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-fg hover:border-accent/50">{{ a.enabled ? 'Disable' : 'Enable' }}</button>
              <button @click="removeAlert(a)" class="text-muted hover:text-rose-400" title="Delete rule">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- recent alert events -->
      <section v-if="events.length" class="space-y-2">
        <h2 class="text-sm font-semibold text-fg">Recent alert events</h2>
        <div class="overflow-hidden rounded-xl border border-line bg-surface">
          <table class="w-full text-sm">
            <tbody>
              <tr v-for="(e, i) in events" :key="i" class="border-b border-line/60 last:border-0">
                <td class="px-4 py-2.5 w-28">
                  <span class="inline-flex items-center gap-1.5 text-xs font-medium" :class="e.firing ? 'text-red-500' : 'text-accent'">
                    <span class="h-2 w-2 rounded-full" :class="e.firing ? 'bg-red-500' : 'bg-accent'"></span>{{ e.firing ? 'Fired' : 'Recovered' }}
                  </span>
                </td>
                <td class="px-4 py-2.5 text-muted">{{ e.message }}</td>
                <td class="px-4 py-2.5 text-right tabular-nums text-faint">{{ evTime(e.at) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </AppShell>
</template>
