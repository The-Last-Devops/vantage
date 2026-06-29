<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const GIB = 2 ** 30

const page = ref(null) // { data, config }
const data = computed(() => page.value?.data || null)
const config = computed(() => page.value?.config || null)

const draft = ref({}) // table -> retention value being edited (in the tier's unit)
const capGb = ref(10) // cap limit shown/edited in GB
const capEnabled = ref(false)
const msg = ref('')

const { loaded, reload: load } = useCached({
  key: () => 'data-retention',
  load: () => api.get('/api/admin/data'),
  apply: (d) => {
    page.value = d
    draft.value = Object.fromEntries(d.data.retention.map((t) => [t.table, t.value ?? '']))
    capGb.value = +(d.data.cap.limit_bytes / GIB).toFixed(2)
    capEnabled.value = d.data.cap.enabled
  },
  onError: () => { page.value = null },
})
onMounted(() => { if (isAdmin.value) load() })

// ---- cap meter ----
const usedPct = computed(() => {
  const c = data.value?.cap
  if (!c || !c.limit_bytes) return 0
  return Math.min(100, (c.used_bytes / c.limit_bytes) * 100)
})
const meterColor = computed(() => (usedPct.value > 90 ? 'bg-down' : usedPct.value >= 70 ? 'bg-warn' : 'bg-accent'))
const fmtBytes = (n) => {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n, i = 0
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(1)} ${u[i]}`
}

async function saveCap() {
  msg.value = ''
  const gb = Number(capGb.value)
  if (!Number.isFinite(gb) || gb < 0.25 || gb > 1024) { msg.value = 'Limit must be between 0.25 and 1024 GB.'; return }
  try {
    await api.post('/api/admin/data-cap', { limit_bytes: Math.round(gb * GIB), enabled: capEnabled.value })
    msg.value = `✓ Cap ${capEnabled.value ? 'enabled' : 'disabled'} at ${gb} GB.`
    await load()
  } catch (e) { msg.value = `Failed (${e.status}).` }
}

// ---- retention tiers, grouped + joined to size/rows ----
const GROUPS = [
  { label: 'System metrics', match: (t) => t.table.startsWith('system_metrics') },
  { label: 'Containers', match: (t) => t.table.startsWith('container_metrics') },
  { label: 'Health', match: (t) => t.table === 'heartbeats' },
]
const sizeByLabel = computed(() => Object.fromEntries((data.value?.tables || []).map((t) => [t.name, t])))
const tierGroups = computed(() => {
  const r = data.value?.retention || []
  return GROUPS.map((g) => ({ label: g.label, tiers: r.filter(g.match) })).filter((g) => g.tiers.length)
})

async function save(tier) {
  msg.value = ''
  const value = Number(draft.value[tier.table])
  if (!Number.isFinite(value) || value < 1) { msg.value = `${tier.label}: enter a positive number of ${tier.unit}.`; return }
  try { await api.post('/api/admin/retention', { table: tier.table, value }); msg.value = `✓ ${tier.label} retention set to ${value} ${tier.unit}.`; await load() }
  catch (e) { msg.value = `Failed (${e.status}).` }
}

const TH = 'border-b border-line2 bg-head px-4 py-3 text-xs font-extrabold uppercase tracking-wide text-fg'
</script>

<template>
  <AppShell title="Data & retention">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage data retention.
    </div>
    <div v-else class="space-y-8">
      <PageLoader v-if="!loaded" />
      <template v-else-if="page">
        <!-- ============ vantage_data (Data DB) ============ -->
        <section class="space-y-4">
          <div class="flex flex-wrap items-center gap-3">
            <span class="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-line bg-surface2 text-accent"><VIcon name="disk" :size="18" /></span>
            <h2 class="font-mono text-h2 text-fg">vantage_data</h2>
            <span class="rounded-pill bg-surface2 px-2 py-0.5 text-micro uppercase tracking-wide text-muted">TimescaleDB · time-series</span>
            <span class="ml-auto font-mono text-metric text-fg">{{ data.db_size }}</span>
          </div>

          <!-- cap card -->
          <div class="space-y-3 rounded-xl border border-line bg-surface p-4">
            <div class="flex items-center justify-between gap-3">
              <div>
                <div class="text-sm font-semibold text-fg">Storage cap</div>
                <div class="text-xs text-faint">{{ fmtBytes(data.cap.used_bytes) }} of {{ fmtBytes(data.cap.limit_bytes) }} used ({{ Math.round(usedPct) }}%)</div>
              </div>
              <label class="flex items-center gap-2 text-sm text-fg">
                <input v-model="capEnabled" type="checkbox" class="h-4 w-4" />Auto-evict
              </label>
            </div>
            <div class="h-2 overflow-hidden rounded bg-line">
              <div class="h-full transition-all" :class="meterColor" :style="{ width: usedPct + '%' }"></div>
            </div>
            <div class="flex flex-wrap items-end gap-3">
              <label class="text-xs text-muted">Limit (GB)
                <input v-model.number="capGb" type="number" min="0.25" max="1024" step="0.25" class="mt-1 block w-28 rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
              </label>
              <button @click="saveCap" class="rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Save</button>
            </div>
            <p class="flex items-start gap-1.5 text-xs text-warn"><VIcon name="alert-triangle" :size="14" class="mt-0.5 shrink-0" />Eviction drops the oldest time chunks first (across every tier) when usage exceeds the limit — the farthest-back history is lost first. Disabled by default.</p>
          </div>

          <!-- tier table (grouped) -->
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <table class="w-full text-sm">
              <thead><tr class="text-left">
                <th :class="TH">Tier</th>
                <th :class="TH" class="text-right">Rows</th>
                <th :class="TH" class="text-right">Size</th>
                <th :class="TH">Keep for</th>
                <th :class="TH"></th>
              </tr></thead>
              <tbody>
                <template v-for="g in tierGroups" :key="g.label">
                  <tr class="bg-surface2/40"><td colspan="5" class="px-4 py-1.5 text-micro font-bold uppercase tracking-wider text-faint">{{ g.label }}</td></tr>
                  <tr v-for="t in g.tiers" :key="t.table" class="border-b border-line/60 last:border-0">
                    <td class="px-4 py-2.5 text-fg">{{ t.label }}<span class="ml-2 font-mono text-xs text-faint">{{ t.table }}</span></td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums text-muted">{{ (sizeByLabel[t.label]?.rows ?? 0).toLocaleString() }}</td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums text-fg">{{ sizeByLabel[t.label]?.size ?? '—' }}</td>
                    <td class="px-4 py-2.5">
                      <div class="flex items-center gap-1.5">
                        <input v-model.number="draft[t.table]" type="number" min="1" class="w-20 rounded-md border border-line2 bg-surface2 px-2 py-1 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
                        <span class="text-xs text-muted">{{ t.unit }}</span>
                      </div>
                    </td>
                    <td class="px-4 py-2.5 text-right">
                      <button @click="save(t)" class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg hover:border-accent/50">Save</button>
                    </td>
                  </tr>
                </template>
              </tbody>
            </table>
          </div>
          <p v-if="msg" class="text-xs" :class="msg.startsWith('✓') ? 'text-accent' : 'text-down'">{{ msg }}</p>
        </section>

        <!-- ============ vantage_config (Config DB, read-only) ============ -->
        <section class="space-y-4">
          <div class="flex flex-wrap items-center gap-3">
            <span class="grid h-9 w-9 shrink-0 place-items-center rounded-lg border border-line bg-surface2 text-muted"><VIcon name="settings" :size="18" /></span>
            <h2 class="font-mono text-h2 text-fg">vantage_config</h2>
            <span class="rounded-pill bg-surface2 px-2 py-0.5 text-micro uppercase tracking-wide text-muted">PostgreSQL · relational</span>
            <span class="ml-auto font-mono text-metric text-fg">{{ config.db_size }}</span>
          </div>
          <div class="overflow-hidden rounded-xl border border-line bg-surface">
            <table class="w-full text-sm">
              <thead><tr class="text-left">
                <th :class="TH">Table</th>
                <th :class="TH" class="text-right">Rows</th>
                <th :class="TH" class="text-right">Size</th>
              </tr></thead>
              <tbody>
                <tr v-for="t in config.tables" :key="t.name" class="border-b border-line/60 last:border-0">
                  <td class="px-4 py-2.5 font-mono text-fg">{{ t.name }}</td>
                  <td class="px-4 py-2.5 text-right font-mono tabular-nums text-muted">{{ t.rows.toLocaleString() }}</td>
                  <td class="px-4 py-2.5 text-right font-mono tabular-nums text-fg">{{ t.size }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </template>
      <p v-else class="text-sm text-down">Couldn't load data stats.</p>
    </div>
  </AppShell>
</template>
