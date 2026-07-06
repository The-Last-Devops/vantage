<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { minLoad } from '../lib/minLoad'

const route = useRoute()
const router = useRouter()
const id = computed(() => route.params.id)

const loaded = ref(false)
const chan = ref(null) // { id, name, kind, workspace, workspace_id, can_edit, config }
const types = ref([])
const rules = ref([])
const byKind = (k) => types.value.find((t) => t.kind === k)
const meta = computed(() => byKind(chan.value?.kind))

// ---- icons (shared glyph set, keyed by manifest `icon`) ----
const ICONS = {
  telegram: { fill: true, body: '<path d="M21.9 4.3 18.6 20c-.2 1-.9 1.3-1.8.8l-4.9-3.6-2.4 2.3c-.3.3-.5.5-1 .5l.3-5 9.1-8.2c.4-.4-.1-.6-.6-.2L6.2 13.1l-4.8-1.5c-1-.3-1-1 .2-1.5l18.7-7.2c.9-.3 1.6.2 1.3 1.4z"/>' },
  slack: { fill: true, body: '<path d="M5 15a2 2 0 1 1-2-2h2v2zm1 0a2 2 0 0 1 4 0v5a2 2 0 0 1-4 0v-5zM9 5a2 2 0 1 1 2-2v2H9zm0 1a2 2 0 0 1 0 4H4a2 2 0 0 1 0-4h5zm10 4a2 2 0 1 1 2 2h-2v-2zm-1 0a2 2 0 0 1-4 0V5a2 2 0 0 1 4 0v5zm-3 9a2 2 0 1 1-2 2v-2h2zm0-1a2 2 0 0 1 0-4h5a2 2 0 0 1 0 4h-5z"/>' },
  discord: { fill: true, body: '<path d="M20 4.4A19 19 0 0 0 15.3 3l-.2.5c1.7.4 2.5.9 3.4 1.5a13 13 0 0 0-9.9 0c.9-.6 1.8-1.1 3.4-1.5L11.7 3A19 19 0 0 0 7 4.4C4 8.9 3.2 13.3 3.6 17.6a19 19 0 0 0 5.7 2.9l.5-.8c-.8-.3-1.5-.7-2.2-1.1l.5-.4a13.6 13.6 0 0 0 11.6 0l.5.4c-.7.4-1.4.8-2.2 1.1l.5.8a19 19 0 0 0 5.7-2.9c.5-5-.8-9.3-3.6-13.2zM9.3 14.9c-1 0-1.9-1-1.9-2.1s.8-2.1 1.9-2.1 1.9 1 1.9 2.1-.9 2.1-1.9 2.1zm5.4 0c-1 0-1.9-1-1.9-2.1s.8-2.1 1.9-2.1 1.9 1 1.9 2.1-.9 2.1-1.9 2.1z"/>' },
  webhook: { fill: false, body: '<path d="M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 0 1 2 17a4 4 0 0 1 4-4"/><path d="m6 17 3.13-5.78c.53-.97.43-2.22-.21-3.08A4 4 0 1 1 16 4"/><path d="m12 6 3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 0 1 0 8"/>' },
  email: { fill: false, body: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m2 7 10 6 10-6"/>' },
  sms: { fill: false, body: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>' },
  push: { fill: false, body: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>' },
  incident: { fill: false, body: '<path d="m10.29 3.86-8.48 14.7A2 2 0 0 0 3.53 21h16.94a2 2 0 0 0 1.72-2.44L13.71 3.86a2 2 0 0 0-3.42 0Z"/><path d="M12 9v4M12 17h.01"/>' },
  chat: { fill: false, body: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>' },
}
function iconSvg(name, size = 20) {
  const ic = ICONS[name] || ICONS.chat
  const attrs = ic.fill ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" ${attrs}>${ic.body}</svg>`
}

// ---- form ----
const form = ref({ name: '', config: {} })
const revealed = ref({})
const err = ref('')
const saving = ref(false)
const testState = ref('') // '' | run | ok | fail
const canEdit = computed(() => !!chan.value?.can_edit)
const fields = computed(() => meta.value?.fields || [])

function validate() {
  if (!form.value.name.trim()) { err.value = 'Give the channel a name.'; return false }
  for (const f of fields.value) {
    if (f.required && f.type !== 'toggle' && !String(form.value.config[f.key] || '').trim()) { err.value = `${f.label} is required.`; return false }
  }
  err.value = ''; return true
}
async function save() {
  if (!validate()) return
  saving.value = true
  try {
    await api.patch(`/api/channels/${id.value}`, { name: form.value.name.trim(), config: form.value.config })
    err.value = '✓ Saved.'
  } catch (e) { err.value = e.status === 403 ? 'You need editor access.' : `Failed (${e.status}).` }
  finally { saving.value = false }
}
// Test the CURRENT (possibly unsaved) config, scoped to this channel's workspace.
async function sendTest() {
  testState.value = 'run'
  try {
    await api.post(`/api/workspaces/${chan.value.workspace_id}/channels/test`, { kind: chan.value.kind, config: form.value.config })
    testState.value = 'ok'
  } catch { testState.value = 'fail' }
  setTimeout(() => { testState.value = '' }, 4000)
}
async function remove() {
  if (!(await confirm({ title: 'Delete channel?', message: `"${chan.value.name}" — alert rules using it are removed too. This cannot be undone.`, danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/channels/${id.value}`); router.push({ name: 'notifications' }) }
  catch (e) { alert(`Failed (${e.status}).`) }
}

// ---- reach: distinct targets the attached rules cover ----
const services = computed(() => rules.value.filter((r) => r.kind === 'service'))
const hosts = computed(() => rules.value.filter((r) => r.kind === 'host'))

onMounted(async () => {
  const work = (async () => {
    types.value = await api.get('/api/channel-types').catch(() => [])
    const all = await api.get('/api/channels').catch(() => [])
    chan.value = all.find((c) => c.id === id.value) || null
    if (!chan.value) return
    form.value = { name: chan.value.name, config: { ...(chan.value.config || {}) } }
    rules.value = await api.get(`/api/channels/${id.value}/alerts`).catch(() => [])
  })()
  await minLoad(work)
  loaded.value = true
})
</script>

<template>
  <AppShell :breadcrumb="[{ label: 'Channels', to: { name: 'notifications' } }, { label: chan?.name || 'Channel' }]">
    <PageLoader v-if="!loaded" />
    <template v-else-if="!chan">
      <p class="rounded-2xl border border-line bg-surface/50 p-10 text-center text-sm text-muted">Channel not found.</p>
    </template>
    <template v-else>
      <div class="mb-5 flex flex-wrap items-center gap-3">
        <span class="grid h-11 w-11 shrink-0 place-items-center rounded-2xl" :style="{ background: meta?.color || 'rgb(var(--surface2))', color: meta?.fg || 'rgb(var(--fg))' }" v-html="iconSvg(meta?.icon || 'chat', 24)"></span>
        <div>
          <h1 class="text-xl font-bold text-fg">{{ chan.name }}</h1>
          <div class="text-xs text-faint">{{ meta?.name || chan.kind }} · {{ chan.workspace }}</div>
        </div>
        <div class="ml-auto flex items-center gap-2">
          <span v-if="testState === 'ok'" class="text-xs text-accent">✓ Test sent</span>
          <span v-else-if="testState === 'fail'" class="text-xs text-down">✗ Test failed</span>
          <button @click="sendTest" :disabled="testState === 'run'" class="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm font-medium text-fg hover:border-accent/50 disabled:opacity-50">
            <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>{{ testState === 'run' ? 'Sending…' : 'Send test' }}
          </button>
        </div>
      </div>

      <p v-if="!canEdit" class="mb-4 rounded-lg border border-line bg-surface2/50 px-3 py-2.5 text-xs text-muted">View only — this channel belongs to another workspace; secrets are masked. Editors of <b class="text-fg">{{ chan.workspace }}</b> can change it.</p>

      <div class="grid items-start gap-4 lg:grid-cols-[1fr_360px]">
        <!-- configuration form -->
        <div class="rounded-2xl border border-line bg-surface p-5">
          <h2 class="mb-4 text-[11px] font-semibold uppercase tracking-wider text-faint">Configuration</h2>
          <fieldset :disabled="!canEdit" class="space-y-4">
            <label class="block">
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Name</span>
              <input v-model="form.name" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
            </label>
            <div v-for="f in fields" :key="f.key">
              <label v-if="f.type === 'toggle'" class="flex cursor-pointer items-center gap-3">
                <input type="checkbox" class="peer sr-only" v-model="form.config[f.key]" />
                <span class="relative h-[22px] w-10 shrink-0 rounded-full bg-line transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-[18px] after:w-[18px] after:rounded-full after:bg-fg after:transition-transform peer-checked:bg-accent peer-checked:after:translate-x-[18px]"></span>
                <span><span class="block text-sm text-fg">{{ f.label }}</span><span v-if="f.hint" class="block text-xs text-faint">{{ f.hint }}</span></span>
              </label>
              <label v-else class="block">
                <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">{{ f.label }}<span v-if="f.required" class="ml-0.5 text-down">*</span></span>
                <UiSelect v-if="f.type === 'select'" v-model="form.config[f.key]" block :options="f.options" />
                <textarea v-else-if="f.type === 'textarea'" v-model="form.config[f.key]" :placeholder="f.placeholder" rows="3" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none"></textarea>
                <span v-else-if="f.type === 'secret'" class="relative block">
                  <input :type="revealed[f.key] ? 'text' : 'password'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 pr-10 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                  <button type="button" @click="revealed[f.key] = !revealed[f.key]" class="absolute right-1.5 top-1.5 rounded p-1.5 text-faint hover:text-fg" :class="revealed[f.key] && 'text-accent'" aria-label="Show"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg></button>
                </span>
                <input v-else :type="f.type === 'number' ? 'number' : 'text'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                <span v-if="f.hint" class="mt-1.5 block text-xs text-faint">{{ f.hint }}</span>
              </label>
            </div>
          </fieldset>
          <div v-if="canEdit" class="mt-5 flex items-center gap-2.5 border-t border-line pt-4">
            <button @click="remove" class="rounded-lg border border-down/35 px-3 py-2 text-xs font-medium text-down hover:bg-down/10">Delete</button>
            <span v-if="err" class="text-xs" :class="err.startsWith('✓') ? 'text-accent' : 'text-down'">{{ err }}</span>
            <span class="ml-auto"></span>
            <button @click="save" :disabled="saving" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ saving ? 'Saving…' : 'Save changes' }}</button>
          </div>
        </div>

        <!-- wiring: rules + reach -->
        <div class="space-y-4">
          <!-- left = rules using it -->
          <div class="rounded-2xl border border-line bg-surface p-5">
            <h2 class="mb-3 text-[11px] font-semibold uppercase tracking-wider text-faint">Used by {{ rules.length }} rule{{ rules.length === 1 ? '' : 's' }}</h2>
            <p v-if="!rules.length" class="text-xs text-faint">No alert rules notify through this channel yet.</p>
            <div v-else class="space-y-1.5">
              <RouterLink v-for="r in rules" :key="r.id" :to="{ name: 'alerts', query: { ws: r.workspace, rule: r.id } }" class="flex items-center gap-2 rounded-lg border border-line bg-surface2 px-2.5 py-2 text-xs hover:border-accent/50">
                <span class="h-1.5 w-1.5 shrink-0 rounded-full" :class="!r.enabled ? 'bg-faint' : r.firing === true ? 'bg-down' : r.firing === false ? 'bg-accent' : 'bg-warn'"></span>
                <span class="truncate text-fg">{{ r.target }}</span>
                <span class="shrink-0 text-faint">· {{ r.workspace }}</span>
                <span v-if="!r.enabled" class="ml-auto shrink-0 text-faint">disabled</span>
              </RouterLink>
            </div>
          </div>

          <!-- right = services / hosts it reaches -->
          <div class="rounded-2xl border border-line bg-surface p-5">
            <h2 class="mb-3 text-[11px] font-semibold uppercase tracking-wider text-faint">Reaches</h2>
            <p v-if="!rules.length" class="text-xs text-faint">Attach this channel to a rule and the services &amp; hosts it covers show here.</p>
            <template v-else>
              <div v-if="services.length" class="mb-3">
                <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-faint">Services · {{ services.length }}</div>
                <div class="flex flex-wrap gap-1.5">
                  <span v-for="r in services" :key="r.id" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-fg">{{ r.target }}</span>
                </div>
              </div>
              <div v-if="hosts.length">
                <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-faint">Hosts · {{ hosts.length }}</div>
                <div class="flex flex-wrap gap-1.5">
                  <span v-for="r in hosts" :key="r.id" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-fg">{{ r.target }}</span>
                </div>
              </div>
            </template>
          </div>
        </div>
      </div>
    </template>
  </AppShell>
</template>
