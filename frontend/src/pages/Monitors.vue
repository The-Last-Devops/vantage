<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'

const route = useRoute()
const selectedNsName = () => {
  const sel = (route.query.ns || '').split(',').filter(Boolean)
  return sel.length === 1 ? sel[0] : null
}

const monitors = ref([])
const namespaces = ref([])
const loading = ref(true)
const err = ref('')
let timer = null

const KINDS = [
  { v: 'http', label: 'HTTP(s)', ph: 'https://example.com/health' },
  { v: 'keyword', label: 'HTTP keyword', ph: 'https://example.com' },
  { v: 'tcp', label: 'TCP port', ph: 'host:port' },
  { v: 'ping', label: 'Ping', ph: 'host or IP' },
]
const kindLabel = (k) => KINDS.find((x) => x.v === k)?.label || k
const isHttp = (k) => k === 'http' || k === 'keyword'

async function load() {
  try { monitors.value = await api.get('/api/monitors'); err.value = '' }
  catch { if (!monitors.value.length) err.value = 'Failed to load monitors' }
  loading.value = false
}

// ---- create / edit form ----
const blank = () => ({
  id: null, name: '', kind: 'http', target: '', nsId: '', interval_secs: 60, timeout_secs: 15, retries: 0, upside_down: false,
  method: 'GET', accepted_status: '', max_redirects: 10, ignore_tls: false, headersText: '', body: '',
  authType: 'none', authUser: '', authPass: '', authToken: '', keyword: '', keyword_invert: false, tags: '', description: '',
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
    keyword: c.keyword || '', keyword_invert: !!c.keyword_invert, tags: (c.tags || []).join(', '), description: c.description || '',
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
  return cfg
}

async function submit() {
  formErr.value = ''
  const v = f.value
  if (!v.name.trim() || !v.target.trim()) { formErr.value = 'Name and target are required.'; return }
  if (v.kind === 'keyword' && !v.keyword.trim()) { formErr.value = 'Keyword is required for keyword monitors.'; return }
  const config = buildConfig()
  try {
    if (isEdit.value) {
      await api.patch(`/api/monitors/${v.id}`, { name: v.name.trim(), target: v.target.trim(), interval_secs: Number(v.interval_secs) || 60, config })
    } else {
      if (!v.nsId) { formErr.value = 'Pick a namespace.'; return }
      await api.post(`/api/namespaces/${v.nsId}/monitors`, { name: v.name.trim(), kind: v.kind, target: v.target.trim(), interval_secs: Number(v.interval_secs) || 60, config })
    }
    formOpen.value = false; await load()
  } catch (e) { formErr.value = e.status === 403 ? 'You need editor access to that namespace.' : `Failed (${e.status}).` }
}
async function removeMonitor(m) {
  if (!confirm(`Delete monitor "${m.name}"?`)) return
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
  <AppShell title="Services">
    <div class="space-y-4">
      <div class="flex items-center justify-between gap-3">
        <p class="text-sm text-muted">Service checks — HTTP / TCP / ping / keyword. Status comes from the latest heartbeat.</p>
        <button @click="formOpen ? (formOpen = false) : openCreate()" class="flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> Add monitor
        </button>
      </div>

      <!-- create / edit form -->
      <form v-if="formOpen" @submit.prevent="submit" class="space-y-4 rounded-xl border border-line bg-surface p-4">
        <div class="text-sm font-semibold text-fg">{{ isEdit ? 'Edit monitor' : 'New monitor' }}</div>

        <!-- general -->
        <div class="flex flex-wrap gap-2">
          <label class="flex-1 text-xs text-faint">Name<input v-model="f.name" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Type<select v-model="f.kind" :disabled="isEdit" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none disabled:opacity-60"><option v-for="k in KINDS" :key="k.v" :value="k.v">{{ k.label }}</option></select></label>
          <label v-if="!isEdit" class="text-xs text-faint">Namespace<select v-model="f.nsId" class="mt-1 block rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none"><option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option></select></label>
        </div>
        <label class="block text-xs text-faint">Target<input v-model="f.target" :placeholder="KINDS.find((k) => k.v === f.kind)?.ph" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>

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

        <!-- http options -->
        <details v-if="isHttp(f.kind)" class="rounded-lg border border-line bg-surface2/40 p-3">
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

      <p v-if="loading" class="text-sm text-muted">Loading…</p>
      <p v-else-if="err" class="text-sm text-rose-400">{{ err }}</p>
      <p v-else-if="!monitors.length" class="rounded-xl border border-line bg-surface p-6 text-center text-sm text-muted">No monitors yet. Add a service check above.</p>

      <div v-else class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">Status</th>
            <th class="px-4 py-3 font-medium">Name</th>
            <th class="px-4 py-3 font-medium">Type</th>
            <th class="px-4 py-3 font-medium">Target</th>
            <th class="px-4 py-3 font-medium text-right">Latency</th>
            <th class="px-4 py-3 font-medium text-right">Last check</th>
            <th class="px-4 py-3"></th>
          </tr></thead>
          <tbody>
            <template v-for="m in monitors" :key="m.id">
            <tr class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3">
                <span v-if="!m.enabled" class="inline-flex items-center gap-1.5 text-xs font-medium text-faint"><span class="h-2 w-2 rounded-full bg-faint"></span>Paused</span>
                <span v-else class="inline-flex items-center gap-1.5 text-xs font-medium" :class="statusOf(m) === 'up' ? 'text-accent' : statusOf(m) === 'down' ? 'text-red-500' : 'text-muted'">
                  <span class="h-2 w-2 rounded-full" :class="statusOf(m) === 'up' ? 'bg-accent' : statusOf(m) === 'down' ? 'bg-red-500' : 'bg-faint'"></span>
                  {{ statusOf(m) === 'up' ? 'Up' : statusOf(m) === 'down' ? 'Down' : 'Pending' }}
                </span>
              </td>
              <td class="px-4 py-3 text-fg">{{ m.name }}<div v-if="m.message" class="text-xs text-faint">{{ m.message }}</div></td>
              <td class="px-4 py-3 text-muted">{{ kindLabel(m.kind) }}</td>
              <td class="px-4 py-3 font-mono text-xs text-muted">{{ m.target }}</td>
              <td class="px-4 py-3 text-right tabular-nums text-muted">{{ m.latency_ms != null ? m.latency_ms + ' ms' : '—' }}</td>
              <td class="px-4 py-3 text-right tabular-nums text-muted">{{ fmtAgo(m.last_check) }}</td>
              <td class="px-4 py-3">
                <div class="flex items-center justify-end gap-3">
                  <button @click="toggleDebug(m)" :class="debugOpen === m.id ? 'text-accent' : 'text-muted hover:text-accent'" title="Debug (last request/response)">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m8 3 4 8 5-5 5 15H2L8 3z"/></svg>
                  </button>
                  <button @click="openEdit(m)" class="text-muted hover:text-accent" title="Edit monitor">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
                  </button>
                  <button @click="removeMonitor(m)" title="Delete monitor" class="text-muted hover:text-rose-400">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                  </button>
                </div>
              </td>
            </tr>
            <tr v-if="debugOpen === m.id" class="border-b border-line/60 bg-surface2/40">
              <td colspan="7" class="px-4 py-4">
                <div v-if="!debugData" class="text-sm text-muted">Loading…</div>
                <div v-else class="grid gap-4 lg:grid-cols-2">
                  <div>
                    <div class="mb-1 flex items-center justify-between">
                      <span class="text-xs font-medium text-accent">Last success</span>
                      <button v-if="debugData.ok" @click="copyDebug(debugData.ok, $event)" class="rounded-md border border-line bg-surface px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
                    </div>
                    <pre v-if="debugData.ok" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debugData.ok) }}</pre>
                    <p v-else class="text-xs text-faint">No successful check recorded yet.</p>
                  </div>
                  <div>
                    <div class="mb-1 flex items-center justify-between">
                      <span class="text-xs font-medium text-red-400">Last failure</span>
                      <button v-if="debugData.err" @click="copyDebug(debugData.err, $event)" class="rounded-md border border-line bg-surface px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
                    </div>
                    <pre v-if="debugData.err" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debugData.err) }}</pre>
                    <p v-else class="text-xs text-faint">No failure recorded.</p>
                  </div>
                </div>
              </td>
            </tr>
            </template>
          </tbody>
        </table>
      </div>
    </div>
  </AppShell>
</template>
