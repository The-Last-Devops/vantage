<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { minLoad } from '../lib/minLoad'

const route = useRoute()
const selectedNsName = () => {
  const sel = (route.query.ns || '').split(',').filter(Boolean)
  return sel.length === 1 ? sel[0] : null
}

const monitors = ref([])
const namespaces = ref([])
const events = ref([])
const loading = ref(true)
const err = ref('')
let timer = null

const KINDS = [
  { v: 'http', label: 'HTTP(s)', ph: 'https://example.com/health' },
  { v: 'keyword', label: 'HTTP keyword', ph: 'https://example.com' },
  { v: 'tcp', label: 'TCP port', ph: 'host:port' },
  { v: 'ping', label: 'Ping', ph: 'host or IP' },
  { v: 'postgres', label: 'PostgreSQL', ph: 'postgres://user:pass@host:5432/db' },
  { v: 'mysql', label: 'MySQL', ph: 'mysql://user:pass@host:3306/db' },
  { v: 'mongodb', label: 'MongoDB', ph: 'mongodb://user:pass@host:27017' },
  { v: 'redis', label: 'Redis', ph: 'host:6379' },
  { v: 'rabbitmq', label: 'RabbitMQ', ph: 'host:5672' },
  { v: 'dns', label: 'DNS', ph: 'example.com' },
  { v: 'tls', label: 'TLS cert', ph: 'host:443' },
  { v: 'push', label: 'Push (passive)', ph: '' },
]
const kindLabel = (k) => KINDS.find((x) => x.v === k)?.label || k
const isHttp = (k) => k === 'http' || k === 'keyword'
const pushUrl = (m) => `${location.origin}/pub/push/${m.config?.push_token || ''}`

// "Down" sub-view (/monitors?status=down) shows only enabled monitors that are down.
const downOnly = computed(() => route.query.status === 'down')
const shown = computed(() => (downOnly.value ? monitors.value.filter((m) => m.enabled && m.up === false) : monitors.value))

// Quick stats (Uptime-Kuma style) across the active monitors.
const stats = computed(() => {
  let up = 0, down = 0, paused = 0
  for (const m of monitors.value) {
    if (!m.enabled) paused++
    else if (m.up === true) up++
    else if (m.up === false) down++
  }
  return { up, down, paused, total: monitors.value.length }
})
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
const monName = (id) => monitors.value.find((m) => m.id === id)?.name || id.slice(0, 8)

// left-list search + rough per-row uptime% from the recent-beats window
const q = ref('')
const filtered = computed(() => {
  const t = q.value.trim().toLowerCase()
  return t ? shown.value.filter((m) => m.name.toLowerCase().includes(t) || (m.target || '').toLowerCase().includes(t)) : shown.value
})
const upPct = (m) => (m.recent && m.recent.length ? Math.round((m.recent.filter(Boolean).length / m.recent.length) * 100) : null)

async function load() {
  const first = loading.value
  try { const w = api.get('/api/monitors'); monitors.value = await (first ? minLoad(w) : w); err.value = '' }
  catch { if (!monitors.value.length) err.value = 'Failed to load monitors' }
  try { events.value = await api.get('/api/events?range=7d') } catch { events.value = [] }
  loading.value = false
}

// ---- create / edit form ----
const blank = () => ({
  id: null, name: '', kind: 'http', target: '', nsId: '', interval_secs: 60, timeout_secs: 15, retries: 1, upside_down: false,
  method: 'GET', accepted_status: '', max_redirects: 10, ignore_tls: false, headersText: '', body: '',
  authType: 'none', authUser: '', authPass: '', authToken: '', keyword: '', keyword_invert: false,
  password: '', expected_ip: '', cert_warn_days: 14, tags: '', description: '',
})
const f = ref(blank())
const formOpen = ref(false)
const isEdit = computed(() => f.value.id != null)
const formErr = ref('')

function openCreate() {
  f.value = blank()
  const match = namespaces.value.find((n) => n.name === selectedNsName())
  f.value.nsId = (match || namespaces.value[0])?.id || ''
  formErr.value = ''; formOpen.value = true
}
function openEdit(m) {
  const c = m.config || {}
  const auth = c.auth || {}
  f.value = {
    id: m.id, name: m.name, kind: m.kind, target: m.target, nsId: '', interval_secs: m.interval_secs,
    timeout_secs: c.timeout_secs ?? 15, retries: c.retries ?? 0, upside_down: !!c.upside_down,
    method: c.method || 'GET', accepted_status: c.accepted_status || '', max_redirects: c.max_redirects ?? 10, ignore_tls: !!c.ignore_tls,
    headersText: c.headers ? Object.entries(c.headers).map(([k, v]) => `${k}: ${v}`).join('\n') : '', body: c.body || '',
    authType: auth.type || 'none', authUser: auth.username || '', authPass: auth.password || '', authToken: auth.token || '',
    keyword: c.keyword || '', keyword_invert: !!c.keyword_invert,
    password: c.password || '', expected_ip: c.expected_ip || '', cert_warn_days: c.cert_warn_days ?? 14, tags: (c.tags || []).join(', '), description: c.description || '',
  }
  formErr.value = ''; formOpen.value = true
}

function buildConfig() {
  const v = f.value
  const cfg = {
    timeout_secs: Number(v.timeout_secs) || 15, retries: Number(v.retries) || 0, upside_down: v.upside_down,
    tags: v.tags.split(',').map((s) => s.trim()).filter(Boolean), description: v.description.trim(),
  }
  if (isHttp(v.kind)) {
    cfg.method = v.method
    cfg.accepted_status = v.accepted_status.trim()
    cfg.max_redirects = Number(v.max_redirects) || 0
    cfg.ignore_tls = v.ignore_tls
    const headers = {}
    for (const line of v.headersText.split('\n')) { const i = line.indexOf(':'); if (i > 0) headers[line.slice(0, i).trim()] = line.slice(i + 1).trim() }
    if (Object.keys(headers).length) cfg.headers = headers
    if (v.body.trim()) cfg.body = v.body
    if (v.authType === 'basic') cfg.auth = { type: 'basic', username: v.authUser, password: v.authPass }
    else if (v.authType === 'bearer') cfg.auth = { type: 'bearer', token: v.authToken }
  }
  if (v.kind === 'keyword') { cfg.keyword = v.keyword; cfg.keyword_invert = v.keyword_invert }
  if (v.kind === 'redis' && v.password) cfg.password = v.password
  if (v.kind === 'dns' && v.expected_ip.trim()) cfg.expected_ip = v.expected_ip.trim()
  if (v.kind === 'tls') cfg.cert_warn_days = Number(v.cert_warn_days) || 14
  // push_token is server-owned — never sent from the client; the backend keeps it.
  return cfg
}

async function submit() {
  formErr.value = ''
  const v = f.value
  if (!v.name.trim()) { formErr.value = 'Name is required.'; return }
  if (v.kind !== 'push' && !v.target.trim()) { formErr.value = 'Target is required.'; return }
  if (v.kind === 'keyword' && !v.keyword.trim()) { formErr.value = 'Keyword is required for keyword monitors.'; return }
  const target = v.kind === 'push' ? 'push' : v.target.trim()
  const config = buildConfig()
  try {
    if (isEdit.value) {
      await api.patch(`/api/monitors/${v.id}`, { name: v.name.trim(), target, interval_secs: Number(v.interval_secs) || 60, config })
    } else {
      if (!v.nsId) { formErr.value = 'Pick a namespace.'; return }
      await api.post(`/api/namespaces/${v.nsId}/monitors`, { name: v.name.trim(), kind: v.kind, target, interval_secs: Number(v.interval_secs) || 60, config })
    }
    formOpen.value = false; await load()
  } catch (e) { formErr.value = e.status === 403 ? 'You need editor access to that namespace.' : `Failed (${e.status}).` }
}
async function removeMonitor(m) {
  if (!confirm(`Delete service "${m.name}"?`)) return
  try { await api.del(`/api/monitors/${m.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}

// ---- debug view ----
const debugOpen = ref(null)
const debugData = ref(null)
async function toggleDebug(m) {
  if (debugOpen.value === m.id) { debugOpen.value = null; return }
  debugData.value = null; debugOpen.value = m.id
  try { debugData.value = await api.get(`/api/monitors/${m.id}/debug`) } catch { debugData.value = { ok: null, err: null } }
}
const fmtDebug = (d) => JSON.stringify(d?.detail ?? {}, null, 2)
function copyDebug(d, ev) {
  navigator.clipboard?.writeText(fmtDebug(d))
  const b = ev.target; const o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}
function copyText(text, ev) {
  navigator.clipboard?.writeText(text)
  const b = ev.target; const o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}

const statusOf = (m) => (m.up == null ? 'pending' : m.up ? 'up' : 'down')
const fmtAgo = (t) => {
  if (!t) return 'never'
  const s = Math.round((Date.now() - new Date(t).getTime()) / 1000)
  return s < 60 ? `${s}s ago` : s < 3600 ? `${Math.round(s / 60)}m ago` : `${Math.round(s / 3600)}h ago`
}

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  await load()
  timer = setInterval(load, 10000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell :title="downOnly ? 'Services — Down' : 'Services'">
    <PageLoader v-if="loading" />
    <div v-else class="flex gap-4">
      <!-- LEFT: monitor list (Uptime-Kuma style) -->
      <aside class="flex w-[330px] shrink-0 flex-col gap-3">
        <button @click="formOpen ? (formOpen = false) : openCreate()" class="flex items-center justify-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> Add service
        </button>
        <input v-model="q" placeholder="Search…" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        <p v-if="err" class="text-sm text-rose-400">{{ err }}</p>
        <p v-else-if="!filtered.length" class="rounded-xl border border-line bg-surface p-4 text-center text-sm text-muted">{{ downOnly ? 'Nothing down. 🎉' : (q ? 'No matches.' : 'No services yet.') }}</p>
        <div v-else class="space-y-1 overflow-y-auto">
          <div v-for="m in filtered" :key="m.id" class="group relative rounded-lg border border-line bg-surface px-2.5 py-2 hover:border-accent/40">
            <RouterLink :to="{ name: 'monitor', params: { id: m.id } }" class="block">
              <div class="flex items-center gap-2">
                <span class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold tabular-nums"
                  :class="!m.enabled ? 'bg-surface2 text-faint' : upPct(m) == null ? 'bg-surface2 text-muted' : upPct(m) >= 99 ? 'bg-accent/15 text-accent' : upPct(m) >= 90 ? 'bg-amber-500/15 text-amber-400' : 'bg-red-500/15 text-red-400'">
                  {{ !m.enabled ? 'OFF' : upPct(m) == null ? 'N/A' : upPct(m) + '%' }}
                </span>
                <span class="min-w-0 flex-1 truncate text-sm font-medium text-fg group-hover:text-accent" :title="m.name">{{ m.name }}</span>
              </div>
              <div class="mt-1.5 flex items-end gap-px" :title="`last ${m.recent ? m.recent.length : 0} checks`">
                <span v-for="(u, i) in (m.recent || [])" :key="i" class="h-3.5 w-[3px] rounded-sm" :class="u ? 'bg-accent' : 'bg-red-500'"></span>
                <span v-if="!m.recent || !m.recent.length" class="text-[10px] text-faint">no checks yet</span>
              </div>
            </RouterLink>
            <div class="absolute right-1.5 top-1.5 hidden items-center gap-1.5 group-hover:flex">
              <button @click.prevent="openEdit(m)" class="rounded bg-surface2 p-1 text-muted hover:text-accent" title="Edit">
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
              </button>
              <button @click.prevent="removeMonitor(m)" class="rounded bg-surface2 p-1 text-muted hover:text-rose-400" title="Delete">
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
              </button>
            </div>
          </div>
        </div>
      </aside>

      <!-- RIGHT: create/edit form OR overview (quick stats + events) -->
      <div class="min-w-0 flex-1 space-y-4">
      <!-- create / edit form -->
      <form v-if="formOpen" @submit.prevent="submit" class="space-y-4 rounded-xl border border-line bg-surface p-4">
        <div class="text-sm font-semibold text-fg">{{ isEdit ? 'Edit service' : 'New service' }}</div>

        <!-- general — pick the type first, then a (short) name -->
        <div class="flex flex-wrap items-end gap-3">
          <label class="text-xs text-faint">Type<select v-model="f.kind" :disabled="isEdit" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none disabled:opacity-60"><option v-for="k in KINDS" :key="k.v" :value="k.v">{{ k.label }}</option></select></label>
          <label class="text-xs text-faint">Name<input v-model="f.name" placeholder="My service" class="mt-1 block w-64 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
          <label v-if="!isEdit" class="text-xs text-faint">Namespace<select v-model="f.nsId" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option></select></label>
        </div>
        <label v-if="f.kind !== 'push'" class="block text-xs text-faint">Target<input v-model="f.target" :placeholder="KINDS.find((k) => k.v === f.kind)?.ph" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
        <p v-else class="rounded-lg border border-line bg-surface2/40 px-3 py-2 text-xs text-muted">Passive check — a push URL is generated after you create it. Have your job call it on schedule; if no call arrives within the interval, it goes Down.</p>
        <!-- tls -->
        <label v-if="f.kind === 'tls'" class="block w-56 text-xs text-faint">Warn when expiring within (days)<input v-model.number="f.cert_warn_days" type="number" min="1" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>

        <div class="flex flex-wrap gap-3">
          <label class="text-xs text-faint">Interval (s)<input v-model.number="f.interval_secs" type="number" min="5" class="mt-1 block w-24 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Timeout (s)<input v-model.number="f.timeout_secs" type="number" min="1" class="mt-1 block w-24 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Retries<input v-model.number="f.retries" type="number" min="0" class="mt-1 block w-24 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="flex items-center gap-2 self-end pb-2 text-sm text-fg"><input v-model="f.upside_down" type="checkbox" class="h-4 w-4" />Upside-down</label>
        </div>

        <!-- keyword -->
        <div v-if="f.kind === 'keyword'" class="flex flex-wrap items-end gap-3">
          <label class="flex-1 text-xs text-faint">Keyword<input v-model="f.keyword" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="flex items-center gap-2 pb-2 text-sm text-fg"><input v-model="f.keyword_invert" type="checkbox" class="h-4 w-4" />Invert (fail if present)</label>
        </div>
        <!-- redis -->
        <label v-if="f.kind === 'redis'" class="block w-72 text-xs text-faint">Password (optional)<input v-model="f.password" type="password" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
        <!-- dns -->
        <label v-if="f.kind === 'dns'" class="block w-72 text-xs text-faint">Expected IP (optional, substring)<input v-model="f.expected_ip" placeholder="1.2.3.4" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>

        <!-- http options -->
        <details v-if="isHttp(f.kind)" open class="rounded-lg border border-line bg-surface2/40 p-3">
          <summary class="cursor-pointer text-xs uppercase tracking-wider text-faint">HTTP options</summary>
          <div class="mt-3 space-y-3">
            <div class="flex flex-wrap gap-3">
              <label class="text-xs text-faint">Method<select v-model="f.method" class="mt-1 block rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option>GET</option><option>POST</option><option>PUT</option><option>HEAD</option><option>DELETE</option><option>PATCH</option></select></label>
              <label class="text-xs text-faint">Accepted status<input v-model="f.accepted_status" placeholder="200-299,301" class="mt-1 block w-40 rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
              <label class="text-xs text-faint">Max redirects<input v-model.number="f.max_redirects" type="number" min="0" class="mt-1 block w-24 rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
              <label class="flex items-center gap-2 self-end pb-2 text-sm text-fg"><input v-model="f.ignore_tls" type="checkbox" class="h-4 w-4" />Ignore TLS errors</label>
            </div>
            <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <label class="block text-xs text-faint">Headers (one per line, <code>Key: Value</code>)<textarea v-model="f.headersText" rows="5" class="mt-1 block w-full rounded-lg border border-line bg-surface px-3 py-2 font-mono text-xs text-fg focus:border-accent/60 focus:outline-none"></textarea></label>
              <label class="block text-xs text-faint">Body<textarea v-model="f.body" rows="5" class="mt-1 block w-full rounded-lg border border-line bg-surface px-3 py-2 font-mono text-xs text-fg focus:border-accent/60 focus:outline-none"></textarea></label>
            </div>
            <div class="flex flex-wrap items-end gap-3">
              <label class="text-xs text-faint">Auth<select v-model="f.authType" class="mt-1 block rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="none">None</option><option value="basic">Basic</option><option value="bearer">Bearer</option></select></label>
              <template v-if="f.authType === 'basic'">
                <label class="text-xs text-faint">Username<input v-model="f.authUser" class="mt-1 block rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
                <label class="text-xs text-faint">Password<input v-model="f.authPass" type="password" class="mt-1 block rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
              </template>
              <label v-else-if="f.authType === 'bearer'" class="flex-1 text-xs text-faint">Token<input v-model="f.authToken" class="mt-1 block w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
            </div>
          </div>
        </details>

        <!-- meta -->
        <div class="flex flex-wrap gap-3">
          <label class="flex-1 text-xs text-faint">Tags (comma-separated)<input v-model="f.tags" placeholder="prod, api" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
          <label class="flex-1 text-xs text-faint">Description<input v-model="f.description" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
        </div>

        <div class="flex items-center gap-3">
          <button type="submit" class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">{{ isEdit ? 'Save' : 'Create' }}</button>
          <button type="button" @click="formOpen = false" class="text-sm text-muted hover:text-fg">Cancel</button>
          <span v-if="formErr" class="text-xs text-rose-400">{{ formErr }}</span>
        </div>
      </form>

      <!-- overview: quick stats + recent events (shown when not adding/editing) -->
      <template v-else>
        <!-- quick stats -->
        <div v-if="!downOnly" class="grid grid-cols-4 gap-3">
          <div class="rounded-xl border border-line bg-surface px-4 py-3">
            <div class="text-[11px] uppercase tracking-wider text-faint">Up</div>
            <div class="text-2xl font-semibold tabular-nums text-accent">{{ stats.up }}</div>
          </div>
          <div class="rounded-xl border border-line bg-surface px-4 py-3">
            <div class="text-[11px] uppercase tracking-wider text-faint">Down</div>
            <div class="text-2xl font-semibold tabular-nums" :class="stats.down ? 'text-red-500' : 'text-muted'">{{ stats.down }}</div>
          </div>
          <div class="rounded-xl border border-line bg-surface px-4 py-3">
            <div class="text-[11px] uppercase tracking-wider text-faint">Paused</div>
            <div class="text-2xl font-semibold tabular-nums text-faint">{{ stats.paused }}</div>
          </div>
          <div class="rounded-xl border border-line bg-surface px-4 py-3">
            <div class="text-[11px] uppercase tracking-wider text-faint">Total</div>
            <div class="text-2xl font-semibold tabular-nums text-fg">{{ stats.total }}</div>
          </div>
        </div>

        <!-- recent events feed (Uptime-Kuma style) -->
        <section class="space-y-2">
          <h2 class="text-sm font-semibold text-fg">Recent events</h2>
          <p v-if="!events.length" class="rounded-xl border border-line bg-surface p-6 text-center text-sm text-muted">No status changes recorded recently.</p>
        <div v-else class="overflow-x-auto rounded-xl border border-line bg-surface">
          <table class="w-full text-sm">
            <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
              <th class="px-4 py-2.5 font-medium">Status</th>
              <th class="px-4 py-2.5 font-medium">Service</th>
              <th class="px-4 py-2.5 font-medium">When</th>
              <th class="px-4 py-2.5 font-medium">Message</th>
            </tr></thead>
            <tbody>
              <tr v-for="(e, i) in events" :key="i" class="border-b border-line/60 last:border-0 hover:bg-surface2/40">
                <td class="px-4 py-2.5">
                  <span class="inline-flex items-center gap-1.5 text-xs font-medium" :class="e.up ? 'text-accent' : 'text-red-500'">
                    <span class="h-2 w-2 rounded-full" :class="e.up ? 'bg-accent' : 'bg-red-500'"></span>{{ e.up ? 'Up' : 'Down' }}
                  </span>
                </td>
                <td class="px-4 py-2.5">
                  <RouterLink :to="{ name: 'monitor', params: { id: e.monitor_id } }" class="text-fg hover:text-accent hover:underline">{{ e.name }}</RouterLink>
                </td>
                <td class="px-4 py-2.5 tabular-nums text-muted">{{ evTime(e.at) }}</td>
                <td class="px-4 py-2.5 text-muted">{{ e.message || '—' }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        </section>
      </template>
      </div>
    </div>
  </AppShell>
</template>
