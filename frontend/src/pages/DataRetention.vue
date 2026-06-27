<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const stats = ref(null)
const draft = ref({})   // table -> value being edited (in the tier's unit)
const msg = ref('')

const { loaded, reload: load } = useCached({
  key: () => 'data-retention',
  load: () => api.get('/api/admin/data'),
  apply: (d) => {
    stats.value = d
    draft.value = Object.fromEntries(d.retention.map((t) => [t.table, t.value ?? '']))
  },
  onError: () => { stats.value = null },
})
onMounted(() => { if (isAdmin.value) load() })

async function save(tier) {
  msg.value = ''
  const value = Number(draft.value[tier.table])
  if (!Number.isFinite(value) || value < 1) { msg.value = `${tier.label}: enter a positive number of ${tier.unit}.`; return }
  try { await api.post('/api/admin/retention', { table: tier.table, value }); msg.value = `✓ ${tier.label} retention set to ${value} ${tier.unit}.`; await load() }
  catch (e) { msg.value = `Failed (${e.status}).` }
}
</script>

<template>
  <AppShell title="Data & retention">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage data retention.
    </div>
    <div v-else class="space-y-6">
      <PageLoader v-if="!loaded" />
      <template v-else-if="stats">
        <!-- DB size + tables -->
        <section class="space-y-3">
          <div class="flex items-baseline gap-2">
            <h2 class="text-sm font-semibold text-fg">Database</h2>
            <span class="text-2xl font-semibold text-fg">{{ stats.db_size }}</span>
            <span class="text-xs text-muted">total on disk</span>
          </div>
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <table class="w-full text-sm">
              <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
                <th class="px-4 py-3 font-medium">Table</th>
                <th class="px-4 py-3 font-medium text-right">Rows</th>
                <th class="px-4 py-3 font-medium text-right">Size</th>
              </tr></thead>
              <tbody>
                <tr v-for="t in stats.tables" :key="t.name" class="border-b border-line/60 last:border-0">
                  <td class="px-4 py-2.5 font-mono text-fg">{{ t.name }}</td>
                  <td class="px-4 py-2.5 text-right tabular-nums text-muted">{{ t.rows.toLocaleString() }}</td>
                  <td class="px-4 py-2.5 text-right tabular-nums text-fg">{{ t.size }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <!-- retention tiers -->
        <section class="space-y-3">
          <h2 class="text-sm font-semibold text-fg">Retention</h2>
          <p class="text-xs text-faint">How long each tier is kept before TimescaleDB drops it. Lower = less disk. The <b>raw</b> tier is the high-resolution realtime data (kept in <b>hours</b>, 24h by default) — it bounds how far back the detailed per-second charts reach; older ranges read from the rollups.</p>
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <table class="w-full text-sm">
              <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
                <th class="px-4 py-3 font-medium">Tier</th>
                <th class="px-4 py-3 font-medium">Keep for</th>
                <th class="px-4 py-3"></th>
              </tr></thead>
              <tbody>
                <tr v-for="t in stats.retention" :key="t.table" class="border-b border-line/60 last:border-0">
                  <td class="px-4 py-3 text-fg">{{ t.label }}<span class="ml-2 font-mono text-xs text-faint">{{ t.table }}</span></td>
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1.5">
                      <input v-model.number="draft[t.table]" type="number" min="1" class="w-20 rounded-md border border-line bg-surface2 px-2 py-1 text-sm text-fg focus:border-accent/60 focus:outline-none" />
                      <span class="text-xs text-muted">{{ t.unit }}</span>
                    </div>
                  </td>
                  <td class="px-4 py-3 text-right">
                    <button @click="save(t)" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg hover:border-accent/50">Save</button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <p v-if="msg" class="text-xs" :class="msg.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ msg }}</p>
        </section>
      </template>
      <p v-else class="text-sm text-rose-400">Couldn't load data stats.</p>
    </div>
  </AppShell>
</template>
