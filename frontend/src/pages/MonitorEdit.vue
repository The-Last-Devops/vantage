<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { minLoad } from '../lib/minLoad'

const route = useRoute()
const router = useRouter()
const editId = computed(() => route.params.id || null)

const workspaces = ref([])
const loaded = ref(false)
const formErr = ref('')
const saving = ref(false)

// --- quick rule (create only) -------------------------------------------------
// Adding a service and then being told when it breaks is one intent, but it used to be
// two trips: create here, then go to Rules and build a rule by hand. Picking channels
// here creates the obvious rule ("this service is DOWN -> notify these") in the same
// step. Anything more specific still belongs in the rule editor.
const channels = ref([])
const ruleChannels = ref(new Set())
// Set once the monitor exists but its rule could not be created — the form must not be
// resubmitted then, or it would create a second monitor.
const createdOnly = ref(false)
function toggleRuleChan(id) {
  const next = new Set(ruleChannels.value)
  next.has(id) ? next.delete(id) : next.add(id)
  ruleChannels.value = next
}

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
const isHttp = (k) => k === 'http' || k === 'keyword'
const isEdit = computed(() => editId.value != null)

const blank = () => ({
  id: null, name: '', kind: 'http', target: '', wsId: '', interval_secs: 60, timeout_secs: 15, retries: 1, upside_down: false,
  method: 'GET', accepted_status: '', max_redirects: 10, ignore_tls: false, headersText: '', body: '',
  authType: 'none', authUser: '', authPass: '', authToken: '', keyword: '', keyword_invert: false,
  password: '', expected_ip: '', cert_warn_days: 14, tags: '', description: '',
})
const f = ref(blank())

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
  return cfg
}

function back() { router.push({ name: 'monitors', query: route.query.ws ? { ws: route.query.ws } : {} }) }

async function submit() {
  formErr.value = ''
  const v = f.value
  if (!v.name.trim()) { formErr.value = 'Name is required.'; return }
  if (v.kind !== 'push' && !v.target.trim()) { formErr.value = 'Target is required.'; return }
  if (v.kind === 'keyword' && !v.keyword.trim()) { formErr.value = 'Keyword is required for keyword monitors.'; return }
  const target = v.kind === 'push' ? 'push' : v.target.trim()
  const config = buildConfig()
  saving.value = true
  try {
    if (isEdit.value) {
      await api.patch(`/api/monitors/${v.id}`, { name: v.name.trim(), target, interval_secs: Number(v.interval_secs) || 60, config })
    } else {
      if (!v.wsId) { formErr.value = 'Pick a workspace.'; saving.value = false; return }
      const id = await api.post(`/api/workspaces/${v.wsId}/monitors`, { name: v.name.trim(), kind: v.kind, target, interval_secs: Number(v.interval_secs) || 60, config })
      const monitorId = typeof id === 'string' ? id : id?.id
      if (ruleChannels.value.size && monitorId) {
        // The service now exists. If the rule fails we must NOT fall through to a
        // retry that creates a second one — say what happened and stop.
        try {
          await api.post(`/api/workspaces/${v.wsId}/alerts`, {
            monitor_id: monitorId,
            channel_ids: [...ruleChannels.value],
          })
        } catch (e) {
          createdOnly.value = true
          formErr.value = `Service created, but the alert rule failed (${e.status || '?'}). Add it under Alert › Rules.`
          saving.value = false
          return
        }
      }
    }
    back()
  } catch (e) { formErr.value = e.status === 403 ? 'You need editor access to that workspace.' : `Failed (${e.status}).` }
  finally { saving.value = false }
}

onMounted(async () => {
  const work = (async () => {
    workspaces.value = await api.get('/api/workspaces').catch(() => [])
    // Only offered while creating: on an existing service its rules are already listed
    // on the service page, and silently adding more from an edit would be surprising.
    if (!editId.value) channels.value = await api.get('/api/channels').catch(() => [])
    if (editId.value) {
      const all = await api.get('/api/monitors').catch(() => [])
      const m = all.find((x) => x.id === editId.value)
      if (m) {
        const c = m.config || {}
        const auth = c.auth || {}
        f.value = {
          id: m.id, name: m.name, kind: m.kind, target: m.target, wsId: '', interval_secs: m.interval_secs,
          timeout_secs: c.timeout_secs ?? 15, retries: c.retries ?? 0, upside_down: !!c.upside_down,
          method: c.method || 'GET', accepted_status: c.accepted_status || '', max_redirects: c.max_redirects ?? 10, ignore_tls: !!c.ignore_tls,
          headersText: c.headers ? Object.entries(c.headers).map(([k, v]) => `${k}: ${v}`).join('\n') : '', body: c.body || '',
          authType: auth.type || 'none', authUser: auth.username || '', authPass: auth.password || '', authToken: auth.token || '',
          keyword: c.keyword || '', keyword_invert: !!c.keyword_invert,
          password: c.password || '', expected_ip: c.expected_ip || '', cert_warn_days: c.cert_warn_days ?? 14, tags: (c.tags || []).join(', '), description: c.description || '',
        }
      }
    } else {
      const pre = (route.query.ws || '').split(',').filter(Boolean)
      const match = workspaces.value.find((n) => n.name === pre[0])
      f.value.wsId = (match || workspaces.value[0])?.id || ''
    }
  })()
  await minLoad(work)
  loaded.value = true
})
</script>

<template>
  <AppShell :breadcrumb="[{ label: 'Services', to: { name: 'monitors', query: route.query.ws ? { ws: route.query.ws } : {} } }, { label: isEdit ? f.name : 'New service' }]">
    <PageLoader v-if="!loaded" />
    <template v-else>
      <!-- Full width, sections as cards in a 2-up grid on a wide screen. It used to be
           one max-w-[720px] column, which left most of the window empty and pushed the
           HTTP textareas into a slot far smaller than what people paste into them.
           AppShell already caps content at 1440px, so no max-w is needed here. -->
      <form @submit.prevent="submit" class="grid w-full grid-cols-1 items-start gap-4 xl:grid-cols-2">
        <!-- Basics: name is the headline field, then type/workspace, then target -->
        <section class="space-y-3 rounded-2xl border border-line bg-surface p-5">
          <div class="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-faint"><VIcon name="service" :size="13" />Basics</div>
          <label class="block text-xs text-muted">Name<input v-model="f.name" placeholder="My service" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-3 text-base font-medium text-fg placeholder:text-faint focus:border-accent/55 focus:outline-none" /></label>
          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label class="block text-xs text-muted">Type<UiSelect v-model="f.kind" block :disabled="isEdit" class="mt-1" :options="KINDS.map((k) => ({ value: k.v, label: k.label }))" /></label>
            <label v-if="!isEdit" class="block text-xs text-muted">Workspace<UiSelect v-model="f.wsId" block class="mt-1" :options="workspaces.map((n) => ({ value: n.id, label: n.name }))" /></label>
          </div>
          <label v-if="f.kind !== 'push'" class="block text-xs text-muted">Target<input v-model="f.target" :placeholder="KINDS.find((k) => k.v === f.kind)?.ph" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg placeholder:text-faint focus:border-accent/55 focus:outline-none" /></label>
          <p v-else class="rounded-lg border border-line2 bg-surface2/40 px-3 py-2 text-xs text-muted">Passive check — a push URL is generated after you create it. Have your job call it on schedule; if no call arrives within the interval, it goes Down.</p>
          <div v-if="f.kind === 'keyword'" class="flex flex-wrap items-end gap-3">
            <label class="flex-1 text-xs text-muted">Keyword<input v-model="f.keyword" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
            <label class="flex items-center gap-2 pb-2 text-sm text-fg"><input v-model="f.keyword_invert" type="checkbox" class="h-4 w-4" />Invert (fail if present)</label>
          </div>
          <label v-if="f.kind === 'redis'" class="block text-xs text-muted">Password (optional)<input v-model="f.password" type="password" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
          <label v-if="f.kind === 'dns'" class="block text-xs text-muted">Expected IP (optional, substring)<input v-model="f.expected_ip" placeholder="1.2.3.4" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg placeholder:text-faint focus:border-accent/55 focus:outline-none" /></label>
          <label v-if="f.kind === 'tls'" class="block text-xs text-muted">Warn when expiring within (days)<input v-model.number="f.cert_warn_days" type="number" min="1" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
        </section>

        <!-- Schedule -->
        <section class="space-y-3 rounded-2xl border border-line bg-surface p-5">
          <div class="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-faint"><VIcon name="clock" :size="13" />Schedule</div>
          <div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
            <label class="block text-xs text-muted">Interval (s)<input v-model.number="f.interval_secs" type="number" min="5" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
            <label class="block text-xs text-muted">Timeout (s)<input v-model.number="f.timeout_secs" type="number" min="1" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
            <label class="block text-xs text-muted">Retries<input v-model.number="f.retries" type="number" min="0" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
          </div>
          <label class="flex items-center gap-2 text-sm text-fg"><input v-model="f.upside_down" type="checkbox" class="h-4 w-4" />Upside-down</label>
        </section>

        <!-- HTTP options -->
        <section v-if="isHttp(f.kind)" class="space-y-3 rounded-2xl border border-line bg-surface p-5 xl:col-span-2">
          <div class="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-faint"><VIcon name="globe" :size="13" />HTTP options</div>
          <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
            <label class="block text-xs text-muted">Method<UiSelect v-model="f.method" block class="mt-1" :options="['GET', 'POST', 'PUT', 'HEAD', 'DELETE', 'PATCH']" /></label>
            <label class="block text-xs text-muted">Accepted status<input v-model="f.accepted_status" placeholder="200-299,301" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg placeholder:text-faint focus:border-accent/55 focus:outline-none" /></label>
            <label class="block text-xs text-muted">Max redirects<input v-model.number="f.max_redirects" type="number" min="0" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
          </div>
          <label class="flex items-center gap-2 text-sm text-fg"><input v-model="f.ignore_tls" type="checkbox" class="h-4 w-4" />Ignore TLS errors</label>
          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label class="block text-xs text-muted">Headers (one per line, <code>Key: Value</code>)<textarea v-model="f.headersText" rows="9" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-xs text-fg focus:border-accent/55 focus:outline-none"></textarea></label>
            <label class="block text-xs text-muted">Body<textarea v-model="f.body" rows="9" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-xs text-fg focus:border-accent/55 focus:outline-none"></textarea></label>
          </div>
          <div class="flex flex-wrap items-end gap-3">
            <label class="text-xs text-muted">Auth<UiSelect v-model="f.authType" block class="mt-1" :options="[['none', 'None'], ['basic', 'Basic'], ['bearer', 'Bearer']]" /></label>
            <template v-if="f.authType === 'basic'">
              <label class="text-xs text-muted">Username<input v-model="f.authUser" class="mt-1 block rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
              <label class="text-xs text-muted">Password<input v-model="f.authPass" type="password" class="mt-1 block rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
            </template>
            <label v-else-if="f.authType === 'bearer'" class="flex-1 text-xs text-muted">Token<input v-model="f.authToken" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 font-mono text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
          </div>
        </section>

        <!-- Quick rule: create-only -->
        <section v-if="!isEdit" class="space-y-3 rounded-2xl border border-line bg-surface p-5 xl:col-span-2">
          <div class="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-faint"><VIcon name="bell" :size="13" />Alert me when it goes down<span class="font-normal normal-case tracking-normal text-faint/70">· optional</span></div>
          <p v-if="!channels.length" class="text-xs text-faint">No notify channels yet — add one under <RouterLink :to="{ name: 'notifications' }" class="text-accent hover:underline">Alert › Notify channel</RouterLink>, then a rule can be created here.</p>
          <template v-else>
            <p class="text-xs text-faint">Creates one rule: <b class="text-muted">this service is DOWN → notify the channels you pick</b>. Thresholds, re-notify and workspace-wide scope live in the rule editor.</p>
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
              <button v-for="c in channels" :key="c.id" type="button" @click="toggleRuleChan(c.id)"
                class="flex items-center gap-2 rounded-lg border px-3 py-2 text-left"
                :class="ruleChannels.has(c.id) ? 'border-accent/60 bg-accent/8' : 'border-line bg-surface2 hover:border-accent/40'">
                <svg v-if="ruleChannels.has(c.id)" class="h-4 w-4 shrink-0 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M20 6 9 17l-5-5"/></svg>
                <span v-else class="h-4 w-4 shrink-0 rounded border border-line"></span>
                <span class="min-w-0 flex-1 truncate text-sm text-fg">{{ c.name }}</span>
                <!-- Channels are shared across workspaces, so label which one each
                     belongs to (a rule may use any existing channel). -->
                <span class="shrink-0 text-[11px] text-faint">{{ c.kind }} · {{ c.workspace }}</span>
              </button>
            </div>
          </template>
        </section>

        <!-- Meta -->
        <section class="space-y-3 rounded-2xl border border-line bg-surface p-5 xl:col-span-2">
          <div class="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-faint"><VIcon name="filter" :size="13" />Meta</div>
          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label class="block text-xs text-muted">Tags (comma-separated)<input v-model="f.tags" placeholder="prod, api" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/55 focus:outline-none" /></label>
            <label class="block text-xs text-muted">Description<input v-model="f.description" class="mt-1 block w-full rounded-lg border border-line2 bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/55 focus:outline-none" /></label>
          </div>
        </section>

        <!-- Footer -->
        <div class="flex items-center gap-3 rounded-2xl border border-line bg-surface px-5 py-4 xl:col-span-2">
          <button v-if="createdOnly" type="button" @click="back" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Done</button>
          <button v-else type="submit" :disabled="saving" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ saving ? 'Saving…' : isEdit ? 'Save changes' : 'Create service' }}</button>
          <button v-if="!createdOnly" type="button" @click="back" class="text-sm text-muted hover:text-fg">Cancel</button>
          <span v-if="formErr" class="text-xs text-down">{{ formErr }}</span>
        </div>
      </form>
    </template>
  </AppShell>
</template>
