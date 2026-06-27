<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const rows = ref([])
const total = ref(0)
const retention = ref(null) // days | null (forever)

// ---- filters + paging ----
const PAGE = 100
const q = ref('')
const method = ref('')
const status = ref('')
const offset = ref(0)
const page = computed(() => Math.floor(offset.value / PAGE) + 1)
const pages = computed(() => Math.max(1, Math.ceil(total.value / PAGE)))
const showingFrom = computed(() => (total.value ? offset.value + 1 : 0))
const showingTo = computed(() => Math.min(offset.value + PAGE, total.value))

// The fetch is driven by the filters + page, so the cache key includes them all —
// changing any filter is a new key and refetches fresh.
const { loaded, reload: load } = useCached({
  key: () => `audit:${offset.value}:${q.value.trim()}:${method.value}:${status.value}`,
  load: async () => {
    const params = new URLSearchParams({ limit: PAGE, offset: offset.value })
    if (q.value.trim()) params.set('q', q.value.trim())
    if (method.value) params.set('method', method.value)
    if (status.value) params.set('status', status.value)
    return await api.get(`/api/audit?${params}`)
  },
  apply: (r) => { rows.value = r.rows || []; total.value = r.total || 0; retention.value = r.retention_days ?? null },
  onError: () => { rows.value = []; total.value = 0 },
})
onMounted(() => { if (isAdmin.value) load() })

// Reset to the first page whenever a filter changes; debounce the text box.
let deb = null
watch(q, () => { clearTimeout(deb); deb = setTimeout(() => { offset.value = 0; load() }, 300) })
watch([method, status], () => { offset.value = 0; load() })
function prev() { if (offset.value > 0) { offset.value = Math.max(0, offset.value - PAGE); load() } }
function next() { if (offset.value + PAGE < total.value) { offset.value += PAGE; load() } }

// ---- retention ----
const RETENTION = [['', 'Forever'], ['30', '30 days'], ['90', '90 days'], ['180', '180 days'], ['365', '1 year']]
async function setRetention(v) {
  try { await api.put('/api/admin/audit/retention', { days: v ? Number(v) : null }); await load() }
  catch { /* leave current value */ }
}

// ---- presentation ----
const fmt = (s) => { const d = new Date(s); return isNaN(d) ? s : d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }) }
const methodColor = (m) => ({ POST: 'text-accent', PATCH: 'text-amber-400', PUT: 'text-amber-400', DELETE: 'text-red-400' }[m] || 'text-muted')
const statusColor = (s) => (s < 300 ? 'text-accent' : s < 400 ? 'text-amber-400' : 'text-red-400')

const VERB = { POST: 'Create', PATCH: 'Update', PUT: 'Update', DELETE: 'Delete' }
const ACTION = { test: 'Test', upload: 'Upload', restore: 'Restore', run: 'Run', revoke: 'Revoke' }
const ENTITY = {
  channels: 'Notify channel', alerts: 'Alert rule', monitors: 'Monitor', systems: 'System',
  namespaces: 'Namespace', users: 'User', keys: 'Enrollment token', tokens: 'API token',
  members: 'Member', memberships: 'Member', backup: 'Backup', schedule: 'Backup schedule',
  data: 'Data & retention', restore: 'Backup', s3: 'S3 backup', pats: 'API token',
  thresholds: 'Thresholds', retention: 'Audit retention',
}
const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s)
const isId = (s) => /^[0-9a-f]{8}-[0-9a-f]{4}/i.test(s) || /^\d+$/.test(s)
function describe(r) {
  const segs = r.path.replace(/^\/api\//, '').split('/').filter(Boolean)
  const names = segs.filter((s) => !isId(s))
  let verb = VERB[r.method] || r.method
  if (ACTION[names[names.length - 1]]) verb = ACTION[names.pop()]
  const key = names[names.length - 1]
  const entity = ENTITY[key] || cap(key) || 'Resource'
  return `${verb} ${entity}`
}
const decorated = computed(() => rows.value.map((r) => ({ ...r, label: describe(r) })))
const filtered = computed(() => !!(q.value.trim() || method.value || status.value))
</script>

<template>
  <AppShell title="Audit">
    <div v-if="!isAdmin" class="rounded-xl border border-line bg-surface p-6 text-center text-muted">Only system admins can view the audit log.</div>
    <div v-else class="space-y-3">
      <!-- toolbar -->
      <div class="flex flex-wrap items-center gap-2.5">
        <div class="relative min-w-[200px] flex-1">
          <svg class="absolute left-3 top-2.5 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
          <input v-model="q" placeholder="Search user, endpoint or object…" class="w-full rounded-lg border border-line bg-surface2 py-2 pl-9 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        </div>
        <UiSelect v-model="method" :options="[['', 'All actions'], ['POST', 'POST'], ['PATCH', 'PATCH'], ['PUT', 'PUT'], ['DELETE', 'DELETE']]" />
        <UiSelect v-model="status" :options="[['', 'Any result'], ['ok', 'Success'], ['client', 'Client error'], ['server', 'Server error']]" />
        <button @click="load()" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-muted hover:text-accent" v-tip="`Refresh`">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16"/></svg>
        </button>
        <label class="flex items-center gap-2 text-xs text-faint">
          <span>Keep logs</span>
          <UiSelect :model-value="retention ? String(retention) : ''" @update:model-value="setRetention" :options="RETENTION" />
        </label>
      </div>

      <PageLoader v-if="!loaded" />
      <template v-else>
        <div class="overflow-hidden rounded-xl border border-line bg-surface">
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
                <th class="px-4 py-3 font-medium">When</th>
                <th class="px-4 py-3 font-medium">User</th>
                <th class="px-4 py-3 font-medium">Action</th>
                <th class="px-4 py-3 font-medium">Object</th>
                <th class="px-4 py-3 font-medium">Endpoint</th>
                <th class="px-4 py-3 font-medium text-right">Result</th>
              </tr></thead>
              <tbody>
                <tr v-if="!rows.length"><td colspan="6" class="px-4 py-10 text-center text-muted">{{ filtered ? 'No actions match these filters.' : 'No actions logged yet.' }}</td></tr>
                <tr v-for="(r, i) in decorated" :key="i" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
                  <td class="whitespace-nowrap px-4 py-2.5 tabular-nums text-muted">{{ fmt(r.at) }}</td>
                  <td class="px-4 py-2.5 text-fg">{{ r.user_email || '—' }}</td>
                  <td class="whitespace-nowrap px-4 py-2.5 font-medium" :class="methodColor(r.method)">{{ r.label }}</td>
                  <td class="px-4 py-2.5 text-fg">{{ r.object_name || '—' }}</td>
                  <td class="px-4 py-2.5 font-mono text-[11px] text-faint" v-tip="r.path">{{ r.path }}</td>
                  <td class="px-4 py-2.5 text-right tabular-nums" :class="statusColor(r.status)">{{ r.status }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <!-- pagination -->
        <div class="flex items-center gap-3 text-xs text-faint">
          <span v-if="total">Showing {{ showingFrom }}–{{ showingTo }} of {{ total }}</span>
          <div class="ml-auto flex items-center gap-2">
            <button @click="prev" :disabled="offset === 0" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-fg hover:border-accent/50 disabled:opacity-40">‹ Prev</button>
            <span class="tabular-nums text-muted">Page {{ page }} / {{ pages }}</span>
            <button @click="next" :disabled="offset + PAGE >= total" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-fg hover:border-accent/50 disabled:opacity-40">Next ›</button>
          </div>
        </div>
      </template>
    </div>
  </AppShell>
</template>
