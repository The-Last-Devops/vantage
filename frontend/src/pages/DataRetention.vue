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

const draft = ref({}) // data-tier table -> retention value being edited (in the tier's unit)
const cfgDraft = ref({}) // config log table -> retention days being edited
const capGb = ref(10) // cap limit shown/edited in GB
const capEnabled = ref(false)
const msg = ref('')

// Hub-decided push cadence (seconds): host "realtime" + k8s cluster scrape.
const sampleHost = ref(5)
const sampleKube = ref(15)
async function loadIntervals() {
  try { const r = await api.get('/api/admin/ingest-intervals'); sampleHost.value = r.host; sampleKube.value = r.kube } catch {}
}
async function saveIntervals() {
  msg.value = ''
  const host = Number(sampleHost.value), kube = Number(sampleKube.value)
  if (!Number.isFinite(host) || host < 1 || host > 3600) { msg.value = 'Host interval must be 1–3600 seconds.'; return }
  if (!Number.isFinite(kube) || kube < 5 || kube > 3600) { msg.value = 'Cluster interval must be 5–3600 seconds.'; return }
  try { await api.post('/api/admin/ingest-intervals', { host, kube }); msg.value = `✓ Sampling: hosts every ${host}s, clusters every ${kube}s (agents apply on their next push).` }
  catch (e) { msg.value = `Failed (${e.status}).` }
}

const { loaded, reload: load } = useCached({
  key: () => 'data-retention',
  load: () => api.get('/api/admin/data'),
  apply: (d) => {
    page.value = d
    draft.value = Object.fromEntries(d.data.retention.map((t) => [t.table, t.value ?? '']))
    cfgDraft.value = Object.fromEntries(
      d.config.tables.filter((t) => t.retention_days != null).map((t) => [t.name, t.retention_days]),
    )
    capGb.value = +(d.data.cap.limit_bytes / GIB).toFixed(2)
    capEnabled.value = d.data.cap.enabled
  },
  onError: () => { page.value = null },
})
onMounted(() => { if (isAdmin.value) { load(); loadIntervals() } })

// Config-DB log tables that have an auto-cleanup window (editable).
const configLogTables = computed(() => (config.value?.tables || []).filter((t) => t.retention_days != null))
const configLogHalves = computed(() => {
  const t = configLogTables.value
  const mid = Math.ceil(t.length / 2)
  return [t.slice(0, mid), t.slice(mid)]
})
async function saveCfg(t) {
  msg.value = ''
  const days = Number(cfgDraft.value[t.name])
  if (!Number.isFinite(days) || days < 1) { msg.value = `${t.name}: enter a positive number of days.`; return }
  try { await api.post('/api/admin/config-retention', { table: t.name, days }); msg.value = `✓ ${t.name} kept ${days} days.`; await load() }
  catch (e) { msg.value = `Failed (${e.status}).` }
}

// ---- cap meter ----
// True usage ratio (can exceed 100%) for the label; the bar width is clamped to 100%.
const usedPct = computed(() => {
  const c = data.value?.cap
  if (!c || !c.limit_bytes) return 0
  return (c.used_bytes / c.limit_bytes) * 100
})
const barPct = computed(() => Math.min(100, usedPct.value))
const overCap = computed(() => usedPct.value > 100)
// How far over the limit, e.g. "3.0×", shown when over cap.
const overFactor = computed(() => {
  const c = data.value?.cap
  return c && c.limit_bytes ? (c.used_bytes / c.limit_bytes).toFixed(1) : '0'
})
const meterColor = computed(() => (usedPct.value >= 90 ? 'bg-down' : usedPct.value >= 70 ? 'bg-warn' : 'bg-accent'))
const fmtBytes = (n) => {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n, i = 0
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(1)} ${u[i]}`
}

const evicting = ref(false)
async function enforceNow() {
  msg.value = ''
  evicting.value = true
  try {
    const r = await api.post('/api/admin/data-cap/enforce', {})
    msg.value = `✓ Evicted ${fmtBytes(r.freed_bytes)} — now ${fmtBytes(r.used_bytes)} / ${fmtBytes(r.limit_bytes)}.`
    await load()
  } catch (e) { msg.value = `Evict failed (${e.status}).` }
  finally { evicting.value = false }
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
  { label: 'Kubernetes', match: (t) => t.table.startsWith('kube_') },
]
const sizeByLabel = computed(() => Object.fromEntries((data.value?.tables || []).map((t) => [t.name, t])))
// Split the config tables into two columns so the (short) list fills the width.
const configHalves = computed(() => {
  const t = config.value?.tables || []
  const mid = Math.ceil(t.length / 2)
  return [t.slice(0, mid), t.slice(mid)]
})
// Bytes for a tier (via its joined size row) — used for sorting + colouring.
const tierBytes = (t) => sizeByLabel.value[t.label]?.size_bytes ?? 0
const tierGroups = computed(() => {
  const r = data.value?.retention || []
  return GROUPS.map((g) => ({
    // Biggest tier first within each group so the space hogs are obvious.
    label: g.label,
    tiers: r.filter(g.match).slice().sort((a, b) => tierBytes(b) - tierBytes(a)),
  })).filter((g) => g.tiers.length)
})
// pg_size_pretty gives "1929 MB" — group the number part so it reads "1,929 MB".
function withCommas(s) {
  if (!s || s === '—') return s
  const m = String(s).match(/^([\d.]+)\s*(.*)$/)
  if (!m) return s
  const num = m[1].includes('.') ? m[1] : Number(m[1]).toLocaleString()
  return m[2] ? `${num} ${m[2]}` : num
}
// Colour the size so large tiers stand out: red ≥ 1 GiB, amber ≥ 256 MiB.
function sizeClass(bytes) {
  if (bytes >= 2 ** 30) return 'text-down'
  if (bytes >= 256 * 2 ** 20) return 'text-warn'
  return 'text-fg'
}
// Lay the tier groups out in two columns (left: System metrics; right: the rest).
const tierColumns = computed(() => {
  const g = tierGroups.value
  const mid = Math.max(1, Math.floor(g.length / 2))
  return [g.slice(0, mid), g.slice(mid)]
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
              <div class="flex items-center gap-2">
                <span class="text-sm font-semibold text-fg">Storage cap</span>
                <span v-if="overCap" class="rounded-pill bg-down/15 px-2 py-0.5 text-micro font-bold uppercase tracking-wide text-down">Over cap · {{ overFactor }}×</span>
              </div>
              <div class="font-mono text-xs" :class="overCap ? 'text-down' : 'text-faint'">{{ fmtBytes(data.cap.used_bytes) }} of {{ fmtBytes(data.cap.limit_bytes) }} used · {{ usedPct.toFixed(1) }}%</div>
            </div>
            <div class="h-2 overflow-hidden rounded bg-line">
              <div class="h-full transition-all" :class="meterColor" :style="{ width: barPct + '%' }"></div>
            </div>
            <div class="flex flex-wrap items-center justify-between gap-3">
              <label class="flex cursor-pointer items-center gap-3">
                <input v-model="capEnabled" type="checkbox" class="peer sr-only" />
                <span class="relative h-[22px] w-10 shrink-0 rounded-full bg-line transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-[18px] after:w-[18px] after:rounded-full after:bg-white after:shadow-sm after:transition-transform peer-checked:bg-accent peer-checked:after:translate-x-[18px]"></span>
                <span class="text-sm text-fg">Auto-evict oldest data when over cap</span>
              </label>
              <div class="flex items-end gap-2">
                <button v-if="overCap" @click="enforceNow" :disabled="evicting" class="rounded-lg border border-down/50 bg-down/10 px-3.5 py-2 text-sm font-semibold text-down hover:bg-down/20 disabled:opacity-50">{{ evicting ? 'Evicting…' : 'Evict now' }}</button>
                <label class="text-xs text-muted">Limit (GB)
                  <input v-model.number="capGb" type="number" min="0.25" max="1024" step="0.25" class="mt-1 block w-24 rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
                </label>
                <button @click="saveCap" class="rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Save</button>
              </div>
            </div>
            <p class="flex items-start gap-1.5 text-xs text-warn"><VIcon name="alert-triangle" :size="14" class="mt-0.5 shrink-0" />When over cap, eviction reclaims space from the <strong>largest tier first</strong>, dropping its oldest chunks — so a runaway tier shrinks before the rest. This can drop data newer than a tier's Keep-for window above.</p>
          </div>

          <!-- sampling cadence (hub-decided push interval) -->
          <div class="space-y-3 rounded-xl border border-line bg-surface p-4">
            <div class="text-sm font-semibold text-fg">Sampling cadence</div>
            <p class="text-xs text-faint">How often agents push metrics — decided here by the hub and applied on each agent's next push (no redeploy). Lower = finer “realtime”, but more raw rows to store.</p>
            <div class="flex flex-wrap items-end gap-4">
              <label class="text-xs text-muted">Hosts (seconds)
                <input v-model.number="sampleHost" type="number" min="1" max="3600" class="mt-1 block w-24 rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
              </label>
              <label class="text-xs text-muted">Clusters (seconds)
                <input v-model.number="sampleKube" type="number" min="5" max="3600" class="mt-1 block w-24 rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
              </label>
              <button @click="saveIntervals" class="rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Save</button>
            </div>
          </div>

          <!-- tier tables (grouped, two columns) -->
          <div class="grid grid-cols-1 gap-4 xl:grid-cols-2">
            <div v-for="(col, ci) in tierColumns" :key="ci" class="overflow-hidden rounded-xl border border-line bg-surface">
              <table class="w-full text-sm">
                <thead><tr class="text-left">
                  <th :class="TH">Tier</th>
                  <th :class="TH" class="text-right">Rows</th>
                  <th :class="TH" class="text-right">Size</th>
                  <th :class="TH">Keep for</th>
                  <th :class="TH"></th>
                </tr></thead>
                <tbody>
                  <template v-for="g in col" :key="g.label">
                    <tr class="bg-surface2/40"><td colspan="5" class="px-4 py-1.5 text-micro font-bold uppercase tracking-wider text-faint">{{ g.label }}</td></tr>
                    <tr v-for="t in g.tiers" :key="t.table" class="border-b border-line/60 last:border-0 align-top">
                      <td class="px-4 py-2.5">
                        <div class="whitespace-nowrap font-mono text-fg">{{ t.table }}</div>
                        <div class="whitespace-nowrap text-xs text-faint">{{ t.label }}</div>
                      </td>
                      <td class="px-4 py-2.5 text-right font-mono tabular-nums text-muted">{{ (sizeByLabel[t.label]?.rows ?? 0).toLocaleString() }}</td>
                      <td class="px-4 py-2.5 text-right font-mono font-semibold tabular-nums" :class="sizeClass(sizeByLabel[t.label]?.size_bytes ?? 0)">{{ withCommas(sizeByLabel[t.label]?.size ?? '—') }}</td>
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

          <!-- log cleanup: editable retention for the time-growing log tables -->
          <div v-if="configLogTables.length" class="overflow-hidden rounded-xl border border-line bg-surface">
            <div class="flex items-center gap-2 border-b border-line2 bg-head px-4 py-2.5">
              <VIcon name="clock" :size="14" class="text-faint" />
              <span class="text-xs font-extrabold uppercase tracking-wide text-fg">Log cleanup</span>
            </div>
            <div class="grid grid-cols-1 lg:grid-cols-2 lg:divide-x lg:divide-line">
              <table v-for="(half, i) in configLogHalves" :key="i" class="w-full text-sm">
                <tbody>
                  <tr v-for="t in half" :key="t.name" class="border-b border-line/60 last:border-0 align-top">
                    <td class="px-4 py-2.5">
                      <div class="font-mono text-fg">{{ t.name }}</div>
                      <div v-if="t.note" class="mt-0.5 text-xs text-faint">{{ t.note }}</div>
                    </td>
                    <td class="px-4 py-2.5 text-right">
                      <div class="inline-flex items-center gap-1.5">
                        <span class="text-xs text-muted">Keep</span>
                        <input v-model.number="cfgDraft[t.name]" type="number" min="1" class="w-16 rounded-md border border-line2 bg-surface2 px-2 py-1 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" />
                        <span class="text-xs text-muted">days</span>
                        <button @click="saveCfg(t)" class="ml-1 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg hover:border-accent/50">Save</button>
                      </div>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
          <p v-if="msg" class="text-xs" :class="msg.startsWith('✓') ? 'text-accent' : 'text-down'">{{ msg }}</p>

          <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
            <div v-for="(half, i) in configHalves" :key="i" class="overflow-hidden rounded-xl border border-line bg-surface">
              <table class="w-full text-sm">
                <thead><tr class="text-left">
                  <th :class="TH">Table</th>
                  <th :class="TH" class="text-right">Rows</th>
                  <th :class="TH" class="text-right">Size</th>
                </tr></thead>
                <tbody>
                  <tr v-for="t in half" :key="t.name" class="border-b border-line/60 last:border-0 align-top">
                    <td class="px-4 py-2.5">
                      <div class="font-mono text-fg">{{ t.name }}</div>
                      <div v-if="t.note" class="mt-0.5 text-xs text-faint">{{ t.note }}</div>
                    </td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums text-muted">{{ t.rows.toLocaleString() }}</td>
                    <td class="px-4 py-2.5 text-right font-mono tabular-nums" :class="sizeClass(t.size_bytes ?? 0)">{{ withCommas(t.size) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </section>
      </template>
      <p v-else class="text-sm text-down">Couldn't load data stats.</p>
    </div>
  </AppShell>
</template>
