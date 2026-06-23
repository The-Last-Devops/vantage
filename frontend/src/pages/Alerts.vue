<script setup>
import { ref, computed, watch, onMounted } from 'vue'
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
const channels = ref([])
const monitors = ref([])
const systems = ref([])
const err = ref('')

const chName = (id) => channels.value.find((c) => c.id === id)?.name || '—'
const target = (a) => {
  if (a.monitor_id) return 'monitor: ' + (monitors.value.find((m) => m.id === a.monitor_id)?.name || a.monitor_id.slice(0, 8))
  if (a.system_id) return 'host: ' + (systems.value.find((s) => s.id === a.system_id)?.name || a.system_id.slice(0, 8))
  return '—'
}
const condText = (a) => {
  const c = a.condition || {}
  if (a.monitor_id) return 'when down'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${c.metric} ${c.op} ${c.value}`
  return '—'
}

async function loadAlerts() {
  if (!nsId.value) { alerts.value = []; return }
  try {
    alerts.value = await api.get(`/api/namespaces/${nsId.value}/alerts`)
    channels.value = await api.get(`/api/namespaces/${nsId.value}/channels`)
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
    body.monitor_id = na.value.monitor_id // fires when the monitor is down
  } else {
    if (!na.value.system_id) { err.value = 'Pick a host.'; return }
    body.system_id = na.value.system_id
    body.condition = na.value.condType === 'offline'
      ? { offline_secs: Number(na.value.offline_secs) || 120 }
      : { metric: na.value.metric, op: na.value.op, value: Number(na.value.value) }
  }
  try { await api.post(`/api/namespaces/${nsId.value}/alerts`, body); await loadAlerts() }
  catch (e) { err.value = e.status === 403 ? 'You need editor access.' : `Failed (${e.status}).` }
}
async function removeAlert(a) {
  try { await api.del(`/api/alerts/${a.id}`); await loadAlerts() } catch (e) { alert(`Failed (${e.status}).`) }
}

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  try { monitors.value = await api.get('/api/monitors') } catch {}
  try { systems.value = await api.get('/api/systems') } catch {}
  const match = namespaces.value.find((n) => n.name === selectedNsName())
  nsId.value = (match || namespaces.value[0])?.id || ''
})
</script>

<template>
  <AppShell title="Alert rules">
    <div class="space-y-5">
      <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold text-fg">Rules</h2>
        <select v-model="nsId" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
          <option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option>
        </select>
      </div>
      <p class="text-xs text-faint">Fire a channel when a monitor goes down or a host breaches a condition. Manage channels under <b>Alert › Notify channel</b>.</p>

      <!-- create -->
      <form @submit.prevent="addAlert" class="space-y-2 rounded-xl border border-line bg-surface p-4">
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

      <!-- list -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">Target</th>
            <th class="px-4 py-3 font-medium">Condition</th>
            <th class="px-4 py-3 font-medium">Channel</th>
            <th class="px-4 py-3 font-medium text-right">Cooldown</th>
            <th class="px-4 py-3"></th>
          </tr></thead>
          <tbody>
            <tr v-if="!alerts.length"><td colspan="5" class="px-4 py-6 text-center text-muted">No alert rules in this namespace.</td></tr>
            <tr v-for="a in alerts" :key="a.id" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3 text-fg">{{ target(a) }}</td>
              <td class="px-4 py-3 text-muted">{{ condText(a) }}</td>
              <td class="px-4 py-3 text-muted">{{ chName(a.channel_id) }}</td>
              <td class="px-4 py-3 text-right tabular-nums text-muted">{{ a.cooldown_secs }}s</td>
              <td class="px-4 py-3 text-right">
                <button @click="removeAlert(a)" title="Delete rule" class="text-muted hover:text-rose-400">
                  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </AppShell>
</template>
