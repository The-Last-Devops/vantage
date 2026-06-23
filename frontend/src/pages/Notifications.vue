<script setup>
import { ref, watch, onMounted } from 'vue'
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
const channels = ref([])
const loading = ref(false)
const err = ref('')

const KINDS = [
  { v: 'telegram', label: 'Telegram', fields: [{ k: 'bot_token', label: 'Bot token' }, { k: 'chat_id', label: 'Chat ID' }] },
  { v: 'webhook', label: 'Webhook', fields: [{ k: 'url', label: 'URL' }] },
  { v: 'email', label: 'Email', fields: [{ k: 'to', label: 'To address' }] },
]
const kindLabel = (k) => KINDS.find((x) => x.v === k)?.label || k

async function loadChannels() {
  if (!nsId.value) { channels.value = []; return }
  loading.value = true
  try { channels.value = await api.get(`/api/namespaces/${nsId.value}/channels`) } catch { channels.value = [] }
  loading.value = false
}
watch(nsId, loadChannels)

const nc = ref({ name: '', kind: 'telegram', config: {} })
watch(() => nc.value.kind, () => { nc.value.config = {} })
const curFields = () => KINDS.find((x) => x.v === nc.value.kind)?.fields || []

async function addChannel() {
  err.value = ''
  if (!nc.value.name.trim()) { err.value = 'Give the channel a name.'; return }
  for (const f of curFields()) if (!nc.value.config[f.k]) { err.value = `${f.label} is required.`; return }
  try {
    await api.post(`/api/namespaces/${nsId.value}/channels`, { name: nc.value.name.trim(), kind: nc.value.kind, config: nc.value.config })
    nc.value = { name: '', kind: nc.value.kind, config: {} }
    await loadChannels()
  } catch (e) { err.value = e.status === 403 ? 'You need editor access to this namespace.' : `Failed (${e.status}).` }
}
async function removeChannel(c) {
  if (!confirm(`Delete channel "${c.name}"? Alert rules using it are removed too.`)) return
  try { await api.del(`/api/channels/${c.id}`); await loadChannels() } catch (e) { alert(`Failed (${e.status}).`) }
}

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
  const match = namespaces.value.find((n) => n.name === selectedNsName())
  nsId.value = (match || namespaces.value[0])?.id || ''
})
</script>

<template>
  <AppShell title="Notifications">
    <div class="space-y-5">
      <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold text-fg">Channels</h2>
        <select v-model="nsId" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
          <option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option>
        </select>
      </div>
      <p class="text-xs text-faint">Where alerts are delivered for this namespace. Add a channel, then attach it to alert rules.</p>

      <!-- create -->
      <form @submit.prevent="addChannel" class="space-y-2 rounded-xl border border-line bg-surface p-4">
        <div class="flex flex-wrap gap-2">
          <input v-model="nc.name" placeholder="channel name (e.g. ops-telegram)" class="min-w-48 flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          <select v-model="nc.kind" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none">
            <option v-for="k in KINDS" :key="k.v" :value="k.v">{{ k.label }}</option>
          </select>
        </div>
        <div class="flex flex-wrap gap-2">
          <input v-for="f in curFields()" :key="f.k" v-model="nc.config[f.k]" :placeholder="f.label"
            class="min-w-48 flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          <button type="submit" class="shrink-0 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">Add channel</button>
        </div>
        <p v-if="err" class="text-xs text-rose-400">{{ err }}</p>
      </form>

      <!-- list -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">Name</th>
            <th class="px-4 py-3 font-medium">Type</th>
            <th class="px-4 py-3"></th>
          </tr></thead>
          <tbody>
            <tr v-if="loading"><td colspan="3" class="px-4 py-6 text-center text-muted">Loading…</td></tr>
            <tr v-else-if="!channels.length"><td colspan="3" class="px-4 py-6 text-center text-muted">No channels in this namespace yet.</td></tr>
            <tr v-for="c in channels" :key="c.id" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3 text-fg">{{ c.name }}</td>
              <td class="px-4 py-3 text-muted">{{ kindLabel(c.kind) }}</td>
              <td class="px-4 py-3 text-right">
                <button @click="removeChannel(c)" title="Delete channel" class="text-muted hover:text-rose-400">
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
