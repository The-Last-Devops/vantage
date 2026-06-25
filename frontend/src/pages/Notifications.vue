<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { minLoad } from '../lib/minLoad'

// Channels are a shared resource: everyone sees every channel (global list); only
// an editor of a channel's own namespace may edit/delete it (the API marks each
// row with `can_edit` and masks secrets for everyone else).
const namespaces = ref([]) // for the "create in namespace" picker
const channels = ref([])
// Start true so the first paint shows the loader, never an empty-state flash.
const loading = ref(true)

// Provider manifest comes from the backend (GET /api/channel-types), so adding a
// provider server-side surfaces here with no frontend change.
const types = ref([])
const byKind = (k) => types.value.find((t) => t.kind === k)
const categories = computed(() => [...new Set(types.value.map((t) => t.category))])

// SVG glyphs keyed by the manifest's `icon` name; generic fallback for the rest.
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
  const attrs = ic.fill
    ? 'fill="currentColor"'
    : 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" ${attrs}>${ic.body}</svg>`
}

async function loadChannels() {
  loading.value = true
  try { channels.value = await minLoad(api.get('/api/channels')) } catch { channels.value = [] }
  finally { loading.value = false }
}

// ---- modal ----
const modalOpen = ref(false)
const step = ref('pick') // 'pick' | 'form'
const cur = ref(null) // current provider meta
const editId = ref(null)
const search = ref('')
const form = ref({ name: '', config: {}, nsId: '' })
const showAdv = ref(false)
const revealed = ref({}) // field key -> bool
const err = ref('')
const modalTest = ref('') // '' | 'run' | 'ok' | 'fail'
const readOnly = ref(false) // viewing a channel you can't edit (other namespace)

const filtered = computed(() => {
  const q = search.value.toLowerCase().trim()
  return categories.value
    .map((cat) => ({
      cat,
      items: types.value.filter(
        (t) => t.category === cat && (!q || t.name.toLowerCase().includes(q) || t.desc.toLowerCase().includes(q)),
      ),
    }))
    .filter((g) => g.items.length)
})
const basicFields = computed(() => cur.value?.fields.filter((f) => !f.advanced) || [])
const advFields = computed(() => cur.value?.fields.filter((f) => f.advanced) || [])

function openNew() {
  editId.value = null; cur.value = null; step.value = 'pick'; readOnly.value = false
  search.value = ''; form.value = { name: '', config: {}, nsId: namespaces.value[0]?.id || '' }
  err.value = ''; modalTest.value = ''; modalOpen.value = true
}
function openEdit(c) {
  const p = byKind(c.kind)
  if (!p) return
  editId.value = c.id; cur.value = p; step.value = 'form'; readOnly.value = false
  form.value = { name: c.name, config: { ...(c.config || {}) }, nsId: c.namespace_id }
  showAdv.value = false; revealed.value = {}; err.value = ''; modalTest.value = ''; modalOpen.value = true
}
// Click a card to view it; opens the editor read-only when you can't edit it.
function openView(c) {
  openEdit(c)
  readOnly.value = !c.can_edit
}
function pickType(p) {
  cur.value = p
  const cfg = {}
  for (const f of p.fields) if (f.default != null) cfg[f.key] = f.default
  form.value = { name: form.value.name, config: cfg, nsId: form.value.nsId }
  showAdv.value = false; revealed.value = {}; err.value = ''; modalTest.value = ''
  step.value = 'form'
}
function backToPick() { step.value = 'pick'; search.value = ''; err.value = '' }
function closeModal() { modalOpen.value = false }

function validate() {
  if (!form.value.name.trim()) { err.value = 'Give the channel a name.'; return false }
  for (const f of cur.value.fields) {
    if (f.required && f.type !== 'toggle' && !String(form.value.config[f.key] || '').trim()) {
      err.value = `${f.label} is required.`; return false
    }
  }
  err.value = ''; return true
}
async function save() {
  if (!validate()) return
  if (!editId.value && !form.value.nsId) { err.value = 'Pick a namespace to create the channel in.'; return }
  const payload = { name: form.value.name.trim(), config: form.value.config }
  try {
    if (editId.value) await api.patch(`/api/channels/${editId.value}`, payload)
    else await api.post(`/api/namespaces/${form.value.nsId}/channels`, { ...payload, kind: cur.value.kind })
    modalOpen.value = false
    await loadChannels()
  } catch (e) {
    err.value = e.status === 403 ? 'You need editor access to this namespace.' : `Failed (${e.status}).`
  }
}
async function modalSendTest() {
  // Only meaningful for an already-saved channel.
  if (!editId.value) { err.value = 'Save the channel first, then send a test.'; return }
  modalTest.value = 'run'
  try { await api.post(`/api/channels/${editId.value}/test`); modalTest.value = 'ok' }
  catch { modalTest.value = 'fail' }
}

// ---- list actions ----
const testState = ref({})
async function testChannel(c) {
  testState.value = { ...testState.value, [c.id]: 'testing' }
  try { await api.post(`/api/channels/${c.id}/test`); testState.value = { ...testState.value, [c.id]: 'ok' } }
  catch { testState.value = { ...testState.value, [c.id]: 'fail' } }
  setTimeout(() => { testState.value = { ...testState.value, [c.id]: undefined } }, 3000)
}
async function removeChannel(c) {
  if (!confirm(`Delete channel "${c.name}"? Alert rules using it are removed too.`)) return
  try { await api.del(`/api/channels/${c.id}`); await loadChannels() } catch (e) { alert(`Failed (${e.status}).`) }
}

onMounted(async () => {
  try { types.value = await api.get('/api/channel-types') } catch { types.value = [] }
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
  await loadChannels()
})
</script>

<template>
  <AppShell title="Notify channels">
    <div class="space-y-5">
      <div class="flex items-start gap-3">
        <p class="text-xs text-faint">Where alerts get delivered. Create a channel once, send a test, then attach it to rules under <b>Alert › Rules</b>.</p>
        <button @click="openNew" class="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-medium text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M12 5v14M5 12h14"/></svg>
          New channel
        </button>
      </div>

      <!-- list -->
      <PageLoader v-if="loading" />
      <div v-else-if="!channels.length" class="flex flex-col items-center gap-3.5 rounded-2xl border border-line bg-surface/50 px-7 py-12 text-center">
        <span class="grid h-16 w-16 place-items-center rounded-2xl border border-accent/30 bg-accent/10 text-accent">
          <svg class="h-7 w-7" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11Z"/></svg>
        </span>
        <h2 class="text-base font-semibold text-fg">No channels yet</h2>
        <p class="max-w-md text-sm text-muted">Connect Telegram, Slack, Discord, email and {{ Math.max(0, types.length - 4) }} other services. Send a test in one click, then route alerts to it.</p>
        <button @click="openNew" class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M12 5v14M5 12h14"/></svg>Create your first channel
        </button>
        <div class="mt-1 flex gap-2">
          <span v-for="p in types.slice(0, 6)" :key="p.kind" class="grid h-9 w-9 place-items-center rounded-xl" :style="{ background: p.color, color: p.fg }" v-html="iconSvg(p.icon, 18)"></span>
        </div>
      </div>
      <div v-else class="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]">
        <div v-for="c in channels" :key="c.id" class="rounded-xl border border-line bg-surface p-3.5 transition-colors hover:border-accent/40">
          <!-- click anywhere on the card to view; actions below require edit rights -->
          <button type="button" @click="openView(c)" class="flex w-full items-center gap-3 text-left">
            <span class="grid h-10 w-10 shrink-0 place-items-center rounded-xl"
              :style="{ background: byKind(c.kind)?.color || 'rgb(var(--surface2))', color: byKind(c.kind)?.fg || 'rgb(var(--fg))' }"
              v-html="iconSvg(byKind(c.kind)?.icon || 'chat', 22)"></span>
            <span class="min-w-0 flex-1">
              <span class="block truncate text-sm font-medium text-fg">{{ c.name }}</span>
              <span class="block text-xs text-faint">
                <span class="mr-1 inline-block h-1.5 w-1.5 rounded-full bg-emerald-400 align-middle"></span>{{ byKind(c.kind)?.name || c.kind }} <span class="text-faint/70">· {{ c.namespace }}</span>
              </span>
            </span>
          </button>
          <div class="mt-3 flex items-center gap-2 border-t border-line/70 pt-3">
            <span v-if="testState[c.id] === 'ok'" class="mr-auto text-xs text-accent">✓ sent</span>
            <span v-else-if="testState[c.id] === 'fail'" class="mr-auto text-xs text-rose-400">✗ failed</span>
            <span v-else class="mr-auto text-[11px] text-faint">{{ c.can_edit ? '' : 'view only' }}</span>
            <template v-if="c.can_edit">
              <button @click="testChannel(c)" :disabled="testState[c.id] === 'testing'" class="rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50">{{ testState[c.id] === 'testing' ? 'Testing…' : 'Test' }}</button>
              <button @click="openEdit(c)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" title="Edit">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
              </button>
              <button @click="removeChannel(c)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-rose-400" title="Delete">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
              </button>
            </template>
          </div>
        </div>
      </div>
    </div>

    <!-- modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="closeModal">
      <div class="w-full max-w-xl overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <span v-if="step === 'form' && cur" class="grid h-9 w-9 shrink-0 place-items-center rounded-xl"
            :style="{ background: cur.color, color: cur.fg }" v-html="iconSvg(cur.icon, 19)"></span>
          <h3 class="text-base font-semibold text-fg">
            {{ step === 'pick' ? 'New notify channel' : (readOnly ? form.name : (editId ? 'Edit ' + form.name : 'New ' + cur?.name + ' channel')) }}
          </h3>
          <button v-if="step === 'form' && !editId" @click="backToPick" class="ml-auto rounded-md px-2.5 py-1.5 text-xs font-semibold text-accent hover:bg-accent/10">Change</button>
          <button @click="closeModal" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" :class="{ 'ml-auto': !(step === 'form' && !editId) }" aria-label="Close">
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg>
          </button>
        </div>

        <!-- STEP 1: pick -->
        <div v-if="step === 'pick'" class="max-h-[60vh] overflow-auto p-5">
          <div class="relative mb-4">
            <svg class="absolute left-3 top-2.5 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
            <input v-model="search" :placeholder="`Search ${types.length} notification types…`" class="w-full rounded-lg border border-line bg-surface2 py-2 pl-9 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </div>
          <div v-for="g in filtered" :key="g.cat">
            <div class="mb-1.5 mt-3 px-1.5 text-[10px] font-bold uppercase tracking-wider text-faint first:mt-0">{{ g.cat }}</div>
            <button v-for="p in g.items" :key="p.kind" @click="pickType(p)" class="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-surface2">
              <span class="grid h-8 w-8 shrink-0 place-items-center rounded-lg" :style="{ background: p.color, color: p.fg }" v-html="iconSvg(p.icon, 17)"></span>
              <span class="min-w-0">
                <span class="block text-sm font-medium text-fg">{{ p.name }}</span>
                <span class="block truncate text-xs text-faint">{{ p.desc }}</span>
              </span>
              <svg class="ml-auto h-4 w-4 shrink-0 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg>
            </button>
          </div>
          <p v-if="!filtered.length" class="py-6 text-center text-sm text-faint">No notification type matches “{{ search }}”.</p>
        </div>

        <!-- STEP 2: configure -->
        <template v-else>
          <fieldset :disabled="readOnly" class="max-h-[60vh] space-y-4 overflow-auto p-5">
            <p v-if="readOnly" class="rounded-lg border border-line bg-surface2/50 px-3 py-2 text-xs text-muted">View only — this channel belongs to another namespace. Editors of that namespace can change it.</p>
            <label v-if="!editId" class="block">
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Namespace</span>
              <select v-model="form.nsId" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
                <option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option>
              </select>
              <span class="mt-1.5 block text-xs text-faint">The channel lives here; only editors of this namespace can change it later. Any alert can still use it.</span>
            </label>
            <label class="block">
              <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Name</span>
              <input v-model="form.name" placeholder="e.g. ops-alerts" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
            </label>

            <!-- basic fields -->
            <div v-for="f in basicFields" :key="f.key">
              <component :is="'div'">
                <!-- toggle -->
                <label v-if="f.type === 'toggle'" class="flex cursor-pointer items-center gap-3">
                  <input type="checkbox" class="peer sr-only" v-model="form.config[f.key]" />
                  <span class="relative h-[22px] w-10 shrink-0 rounded-full bg-line transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-[18px] after:w-[18px] after:rounded-full after:bg-fg after:transition-transform peer-checked:bg-accent peer-checked:after:translate-x-[18px]"></span>
                  <span>
                    <span class="block text-sm text-fg">{{ f.label }}</span>
                    <span v-if="f.hint" class="block text-xs text-faint">{{ f.hint }}</span>
                  </span>
                </label>
                <!-- everything else -->
                <label v-else class="block">
                  <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">{{ f.label }}<span v-if="f.required" class="ml-0.5 text-rose-400">*</span></span>
                  <select v-if="f.type === 'select'" v-model="form.config[f.key]" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
                    <option v-for="o in f.options" :key="o" :value="o">{{ o }}</option>
                  </select>
                  <textarea v-else-if="f.type === 'textarea'" v-model="form.config[f.key]" :placeholder="f.placeholder" rows="3" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none"></textarea>
                  <span v-else-if="f.type === 'secret'" class="relative block">
                    <input :type="revealed[f.key] ? 'text' : 'password'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 pr-10 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                    <button type="button" @click="revealed[f.key] = !revealed[f.key]" class="absolute right-1.5 top-1.5 rounded p-1.5 text-faint hover:text-fg" :class="revealed[f.key] && 'text-accent'" aria-label="Show">
                      <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>
                    </button>
                  </span>
                  <input v-else :type="f.type === 'number' ? 'number' : 'text'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                  <span v-if="f.hint" class="mt-1.5 block text-xs text-faint">{{ f.hint }}</span>
                </label>
              </component>
            </div>

            <!-- advanced (always expanded — plenty of room, nothing to hide) -->
            <div v-if="advFields.length" class="border-t border-line/70 pt-4">
              <div class="mb-3 text-[11px] font-semibold uppercase tracking-wide text-faint">Advanced options <span class="font-normal normal-case text-faint/70">· optional</span></div>
              <div class="space-y-4">
                <div v-for="f in advFields" :key="f.key">
                  <label v-if="f.type === 'toggle'" class="flex cursor-pointer items-center gap-3">
                    <input type="checkbox" class="peer sr-only" v-model="form.config[f.key]" />
                    <span class="relative h-[22px] w-10 shrink-0 rounded-full bg-line transition-colors after:absolute after:left-0.5 after:top-0.5 after:h-[18px] after:w-[18px] after:rounded-full after:bg-fg after:transition-transform peer-checked:bg-accent peer-checked:after:translate-x-[18px]"></span>
                    <span>
                      <span class="block text-sm text-fg">{{ f.label }}</span>
                      <span v-if="f.hint" class="block text-xs text-faint">{{ f.hint }}</span>
                    </span>
                  </label>
                  <label v-else class="block">
                    <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">{{ f.label }}<span v-if="f.required" class="ml-0.5 text-rose-400">*</span></span>
                    <select v-if="f.type === 'select'" v-model="form.config[f.key]" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
                      <option v-for="o in f.options" :key="o" :value="o">{{ o }}</option>
                    </select>
                    <textarea v-else-if="f.type === 'textarea'" v-model="form.config[f.key]" :placeholder="f.placeholder" rows="3" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none"></textarea>
                    <span v-else-if="f.type === 'secret'" class="relative block">
                      <input :type="revealed[f.key] ? 'text' : 'password'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 pr-10 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                      <button type="button" @click="revealed[f.key] = !revealed[f.key]" class="absolute right-1.5 top-1.5 rounded p-1.5 text-faint hover:text-fg" :class="revealed[f.key] && 'text-accent'" aria-label="Show">
                        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>
                      </button>
                    </span>
                    <input v-else :type="f.type === 'number' ? 'number' : 'text'" v-model="form.config[f.key]" :placeholder="f.placeholder" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                    <span v-if="f.hint" class="mt-1.5 block text-xs text-faint">{{ f.hint }}</span>
                  </label>
                </div>
              </div>
            </div>
          </fieldset>

          <div class="flex items-center gap-2.5 border-t border-line bg-surface/60 px-5 py-3.5">
            <button v-if="!readOnly" @click="modalSendTest" :disabled="modalTest === 'run'" class="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface2 px-3 py-2 text-xs font-medium text-fg hover:border-accent/50 disabled:opacity-50">
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>
              Send test
            </button>
            <span v-if="modalTest === 'run'" class="text-xs text-muted">Sending…</span>
            <span v-else-if="modalTest === 'ok'" class="text-xs text-accent">✓ Test sent</span>
            <span v-else-if="modalTest === 'fail'" class="text-xs text-rose-400">✗ Failed to send</span>
            <span v-if="err" class="text-xs text-rose-400">{{ err }}</span>
            <span class="ml-auto"></span>
            <button @click="closeModal" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">{{ readOnly ? 'Close' : 'Cancel' }}</button>
            <button v-if="!readOnly" @click="save" class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">Save channel</button>
          </div>
        </template>
      </div>
    </div>
  </AppShell>
</template>
