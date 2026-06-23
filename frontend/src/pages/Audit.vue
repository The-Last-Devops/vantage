<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)
const rows = ref([])
const loading = ref(true)

async function load() {
  loading.value = true
  try { rows.value = await api.get('/api/audit') } catch { rows.value = [] }
  loading.value = false
}
onMounted(() => { if (isAdmin.value) load() })

const fmt = (s) => { const d = new Date(s); return isNaN(d) ? s : d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }) }
const methodColor = (m) => ({ POST: 'text-accent', PATCH: 'text-amber-400', PUT: 'text-amber-400', DELETE: 'text-red-400' }[m] || 'text-muted')
const statusColor = (s) => (s < 300 ? 'text-accent' : s < 400 ? 'text-amber-400' : 'text-red-400')
</script>

<template>
  <AppShell title="Audit">
    <div v-if="!isAdmin" class="rounded-xl border border-line bg-surface p-6 text-center text-muted">Only system admins can view the audit log.</div>
    <div v-else class="space-y-3">
      <div class="flex items-center gap-2">
        <p class="text-sm text-muted">Every change action (who · what · when · result). Newest first, last 500.</p>
        <button @click="load" class="ml-auto rounded-lg border border-line bg-surface2 px-3 py-1.5 text-xs text-muted hover:text-accent">Refresh</button>
      </div>
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">When</th>
            <th class="px-4 py-3 font-medium">User</th>
            <th class="px-4 py-3 font-medium">Action</th>
            <th class="px-4 py-3 font-medium">Path</th>
            <th class="px-4 py-3 font-medium text-right">Result</th>
          </tr></thead>
          <tbody>
            <tr v-if="loading"><td colspan="5" class="px-4 py-6 text-center text-muted">Loading…</td></tr>
            <tr v-else-if="!rows.length"><td colspan="5" class="px-4 py-6 text-center text-muted">No actions logged yet.</td></tr>
            <tr v-for="(r, i) in rows" :key="i" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-2.5 tabular-nums text-muted">{{ fmt(r.at) }}</td>
              <td class="px-4 py-2.5 text-fg">{{ r.user_email || '—' }}</td>
              <td class="px-4 py-2.5 font-mono text-xs font-medium" :class="methodColor(r.method)">{{ r.method }}</td>
              <td class="px-4 py-2.5 font-mono text-xs text-muted">{{ r.path }}</td>
              <td class="px-4 py-2.5 text-right tabular-nums" :class="statusColor(r.status)">{{ r.status }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </AppShell>
</template>
