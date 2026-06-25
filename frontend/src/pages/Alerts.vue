<script setup>
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { minLoad } from '../lib/minLoad'
import { api } from '../lib/api'

const route = useRoute()
const highlightId = computed(() => route.query.rule || null)

const namespaces = ref([])
// Namespaces in scope: the sidebar selection (?ns=a,b), or all when none chosen.
const selectedNsNames = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const activeNs = computed(() =>
  selectedNsNames.value.length
    ? namespaces.value.filter((n) => selectedNsNames.value.includes(n.name))
    : namespaces.value,
)
const alerts = ref([])
const channels = ref([])
const monitors = ref([])
const systems = ref([])
const types = ref([]) // channel-type manifest, for channel badge colors/icons
const loaded = ref(false) // true after the first load, so polling never re-flashes the loader
let timer = null

// ---- channel badge helpers (kind → color/icon) ----
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
const typeByKind = (k) => types.value.find((t) => t.kind === k)
function iconSvg(name, size = 14) {
  const ic = ICONS[name] || ICONS.chat
  const attrs = ic.fill ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" ${attrs}>${ic.body}</svg>`
}
const chanColor = (kind) => typeByKind(kind)?.color || 'rgb(var(--surface2))'
const chanFg = (kind) => typeByKind(kind)?.fg || 'rgb(var(--fg))'
const chanIcon = (kind) => typeByKind(kind)?.icon || 'chat'

// ---- condition rendering ----
const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function condText(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor') return 'is DOWN'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC_LABEL[c.metric] || c.metric} ${c.op} ${c.value}`
  return '—'
}
function stateOf(a) {
  if (!a.enabled) return 'disabled'
  if (a.firing === true) return 'firing'
  if (a.firing === false) return 'ok'
  return 'pending'
}
const STATE = {
  firing: { label: 'Firing', cls: 'text-red-400 bg-red-500/12', dot: 'bg-red-500' },
  ok: { label: 'OK', cls: 'text-accent bg-accent/12', dot: 'bg-accent' },
  disabled: { label: 'Disabled', cls: 'text-faint bg-surface2', dot: 'bg-faint' },
  pending: { label: 'Pending', cls: 'text-amber-400 bg-amber-400/12', dot: 'bg-amber-400' },
}
const firing = computed(() => alerts.value.filter((a) => stateOf(a) === 'firing'))
const others = computed(() => alerts.value.filter((a) => stateOf(a) !== 'firing'))
const sections = computed(() => {
  const s = []
  if (firing.value.length) s.push({ key: 'firing', label: 'Firing', items: firing.value })
  if (others.value.length) s.push({ key: 'idle', label: 'Healthy & idle', items: others.value })
  return s
})
const renotifyText = (a) => (a.renotify_secs ? `re-notify every ${Math.round(a.renotify_secs / 60)}m` : 'notify once')
function ago(iso) {
  if (!iso) return ''
  let s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000)
  const d = Math.floor(s / 86400); s -= d * 86400
  const h = Math.floor(s / 3600); s -= h * 3600
  const m = Math.floor(s / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  if (m) return `${m}m`
  return `${Math.floor(s)}s`
}

async function load() {
  const nss = activeNs.value
  if (!nss.length) { alerts.value = []; channels.value = []; loaded.value = true; return }
  const first = !loaded.value
  try {
    const work = (async () => {
      // Rules are merged across the namespaces in scope; channels are a shared
      // global resource (any rule may use any channel), fetched once.
      const [aLists, allChannels] = await Promise.all([
        Promise.all(nss.map((n) => api.get(`/api/namespaces/${n.id}/alerts`).catch(() => []))),
        api.get('/api/channels').catch(() => []),
      ])
      alerts.value = aLists.flat()
      channels.value = allChannels
    })()
    await (first ? minLoad(work) : work)
  } catch { alerts.value = [] }
  finally { loaded.value = true }
}
watch(() => route.query.ns, load)
watch([alerts, highlightId], async () => {
  if (!highlightId.value) return
  await nextTick()
  document.getElementById(`rule-${highlightId.value}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' })
})

// ---- list actions ----
async function toggle(a) {
  try { await api.patch(`/api/alerts/${a.id}`, { enabled: !a.enabled }); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}
async function removeAlert(a) {
  if (!confirm('Delete this alert rule?')) return
  try { await api.del(`/api/alerts/${a.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}
const testState = ref({})
async function testAlert(a) {
  testState.value = { ...testState.value, [a.id]: 'testing' }
  try { await api.post(`/api/alerts/${a.id}/test`); testState.value = { ...testState.value, [a.id]: 'ok' } }
  catch { testState.value = { ...testState.value, [a.id]: 'fail' } }
  setTimeout(() => { testState.value = { ...testState.value, [a.id]: undefined } }, 3000)
}

// ---- editor modal ----
const modalOpen = ref(false)
const editId = ref(null)
const err = ref('')
const ed = ref({ srcType: 'monitor', targetId: '', condType: 'metric', metric: 'cpu_percent', op: '>', value: 90, offlineSecs: 120, channels: new Set(), renotify: '' })
// Candidate sources = those in the active namespace(s).
const activeNsNames = computed(() => new Set(activeNs.value.map((n) => n.name)))
const monsInNs = computed(() => monitors.value.filter((m) => activeNsNames.value.has(m.namespace)))
const sysInNs = computed(() => systems.value.filter((s) => activeNsNames.value.has(s.namespace)))
// Namespace of the picked target → where the rule is created (auth = editor there).
const targetNs = computed(() => {
  const list = ed.value.srcType === 'monitor' ? monitors.value : systems.value
  const name = list.find((x) => x.id === ed.value.targetId)?.namespace
  return namespaces.value.find((n) => n.name === name) || null
})
// Channels are global — any rule may notify any channel.
const editChannels = computed(() => channels.value)
const RENOTIFY = [['', 'Off — notify once'], ['900', 'every 15 min'], ['1800', 'every 30 min'], ['3600', 'every hour']]

function openNew() {
  editId.value = null; err.value = ''
  ed.value = { srcType: 'monitor', targetId: '', condType: 'metric', metric: 'cpu_percent', op: '>', value: 90, offlineSecs: 120, channels: new Set(), renotify: '' }
  modalOpen.value = true
}
function openEdit(a) {
  editId.value = a.id; err.value = ''
  const c = a.condition || {}
  ed.value = {
    srcType: a.target_kind === 'monitor' ? 'monitor' : 'host',
    targetId: a.monitor_id || a.system_id || '',
    condType: c.offline_secs ? 'offline' : 'metric',
    metric: c.metric || 'cpu_percent',
    op: c.op || '>',
    value: c.value ?? 90,
    offlineSecs: c.offline_secs ?? 120,
    channels: new Set((a.channels || []).map((ch) => ch.id)),
    renotify: a.renotify_secs ? String(a.renotify_secs) : '',
  }
  modalOpen.value = true
}
function setSrcType(t) {
  ed.value.srcType = t
  ed.value.targetId = ''
  ed.value.condType = t === 'monitor' ? 'down' : 'metric'
}
function toggleChan(id) {
  const s = ed.value.channels
  s.has(id) ? s.delete(id) : s.add(id)
  ed.value.channels = new Set(s)
}

const targetName = computed(() => {
  const list = ed.value.srcType === 'monitor' ? monitors.value : sysInNs.value
  return list.find((x) => x.id === ed.value.targetId)?.name || ''
})
const summary = computed(() => {
  const t = targetName.value || '<source>'
  let cond
  if (ed.value.srcType === 'monitor') cond = 'is down'
  else if (ed.value.condType === 'offline') cond = `is offline for ${ed.value.offlineSecs}s`
  else cond = `${METRIC_LABEL[ed.value.metric]} ${ed.value.op} ${ed.value.value}`
  const names = [...ed.value.channels].map((id) => channels.value.find((c) => c.id === id)?.name).filter(Boolean)
  return { t, cond, names }
})

function buildCondition() {
  if (ed.value.srcType === 'monitor') return {}
  if (ed.value.condType === 'offline') return { offline_secs: Number(ed.value.offlineSecs) || 120 }
  return { metric: ed.value.metric, op: ed.value.op, value: Number(ed.value.value) }
}
async function save() {
  err.value = ''
  if (!editId.value && !ed.value.targetId) { err.value = `Pick a ${ed.value.srcType === 'monitor' ? 'service' : 'host'}.`; return }
  if (!ed.value.channels.size) { err.value = 'Pick at least one channel.'; return }
  const channel_ids = [...ed.value.channels]
  const renotify_secs = ed.value.renotify ? Number(ed.value.renotify) : null
  try {
    if (editId.value) {
      await api.patch(`/api/alerts/${editId.value}`, { channel_ids, renotify_secs, condition: buildCondition() })
    } else {
      if (!targetNs.value) { err.value = 'Pick a source first.'; return }
      const body = { channel_ids, renotify_secs, condition: buildCondition() }
      if (ed.value.srcType === 'monitor') body.monitor_id = ed.value.targetId
      else body.system_id = ed.value.targetId
      await api.post(`/api/namespaces/${targetNs.value.id}/alerts`, body)
    }
    modalOpen.value = false
    await load()
  } catch (e) { err.value = e.status === 403 ? 'You need editor access.' : `Failed (${e.status}).` }
}
async function modalTest() {
  if (!editId.value) { err.value = 'Save the rule first, then test.'; return }
  try { await api.post(`/api/alerts/${editId.value}/test`); err.value = '' } catch (e) { err.value = `Test failed (${e.status}).` }
}

onMounted(async () => {
  try { types.value = await api.get('/api/channel-types') } catch {}
  try { namespaces.value = await api.get('/api/namespaces') } catch {}
  try { monitors.value = await api.get('/api/monitors') } catch {}
  try { systems.value = await api.get('/api/systems') } catch {}
  await load()
  timer = setInterval(load, 15000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Alert rules">
    <div class="space-y-5">
      <div class="flex items-start gap-3">
        <p class="text-xs text-faint">The wiring: a <b>source</b> (a host from Infrastructure or a service from Services) → a <b>condition</b> → the <b>channels</b> that fire. Incidents then land in <b>Alert › Events</b>.</p>
        <button @click="openNew" class="ml-auto flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg> New rule
        </button>
      </div>

      <PageLoader v-if="!loaded" />
      <p v-else-if="!alerts.length" class="rounded-2xl border border-line bg-surface/50 p-10 text-center text-sm text-muted">No alert rules yet. Click <b class="text-fg">New rule</b> to wire one up.</p>

      <section v-for="g in sections" :key="g.key" class="space-y-2.5">
        <div class="flex items-center gap-2 text-[11px] font-bold uppercase tracking-wider text-faint">
          <span v-if="g.key === 'firing'" class="inline-flex items-center gap-1.5 rounded-full bg-red-500/12 px-2 py-0.5 text-red-400"><span class="h-1.5 w-1.5 rounded-full bg-red-500"></span>Firing</span>
          <span v-else>{{ g.label }}</span>
          <span class="rounded-full bg-surface2 px-2 py-0.5 text-[10px]">{{ g.items.length }}</span>
        </div>

        <div v-for="a in g.items" :key="a.id" :id="`rule-${a.id}`" @click="openEdit(a)"
          class="cursor-pointer rounded-2xl border bg-surface p-4 transition-colors hover:border-accent/40"
          :class="[stateOf(a) === 'firing' ? 'border-red-500/45' : 'border-line', String(a.id) === String(highlightId) ? 'ring-2 ring-accent/60' : '', !a.enabled ? 'opacity-60' : '']">
          <div class="mb-3 flex items-center gap-2.5">
            <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-semibold" :class="STATE[stateOf(a)].cls"><span class="h-1.5 w-1.5 rounded-full" :class="STATE[stateOf(a)].dot"></span>{{ STATE[stateOf(a)].label }}</span>
            <span v-if="a.since && stateOf(a) !== 'disabled'" class="text-xs text-faint">{{ stateOf(a) === 'firing' ? 'firing' : 'ok' }} for {{ ago(a.since) }}</span>
            <span class="ml-auto"></span>
            <button @click.stop="toggle(a)" :title="a.enabled ? 'Disable' : 'Enable'" class="relative h-[22px] w-10 shrink-0 rounded-full transition-colors" :class="a.enabled ? 'bg-accent' : 'bg-line'">
              <span class="absolute top-0.5 h-[18px] w-[18px] rounded-full transition-all" :class="a.enabled ? 'left-[20px] bg-accentfg' : 'left-0.5 bg-fg'"></span>
            </button>
          </div>
          <div class="flex flex-wrap items-center gap-2.5">
            <span class="inline-flex items-center gap-2 rounded-lg border border-line bg-bg px-3 py-1.5 text-[13px]">
              <svg v-if="a.target_kind === 'monitor'" class="h-[15px] w-[15px] text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
              <svg v-else class="h-[15px] w-[15px] text-sky-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>
              {{ a.target_name || '—' }}<span class="text-[11px] text-faint"> · {{ a.target_kind === 'monitor' ? 'Service' : 'Host' }}</span>
            </span>
            <span class="inline-flex items-center gap-1 text-[11px] text-faint">when <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></span>
            <span class="inline-flex items-center rounded-lg border border-amber-400/40 bg-amber-400/8 px-3 py-1.5 font-mono text-[13px] text-amber-300">{{ condText(a) }}</span>
            <span class="inline-flex items-center gap-1 text-[11px] text-faint">notify <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg></span>
            <span class="inline-flex flex-wrap items-center gap-1.5">
              <span v-for="ch in a.channels" :key="ch.id" :title="ch.name" class="grid h-[26px] w-[26px] place-items-center rounded-lg" :style="{ background: chanColor(ch.kind), color: chanFg(ch.kind) }" v-html="iconSvg(chanIcon(ch.kind), 15)"></span>
              <span v-if="!a.channels.length" class="text-xs text-rose-400">no channel</span>
            </span>
          </div>
          <div class="mt-3 flex items-center gap-2 border-t border-line/70 pt-3">
            <span class="mr-auto text-xs text-faint">{{ renotifyText(a) }}</span>
            <span v-if="testState[a.id] === 'ok'" class="text-xs text-accent">✓ sent</span>
            <span v-else-if="testState[a.id] === 'fail'" class="text-xs text-rose-400">✗ failed</span>
            <button @click.stop="testAlert(a)" :disabled="testState[a.id] === 'testing'" class="rounded-lg border border-line bg-surface2 px-2.5 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50">{{ testState[a.id] === 'testing' ? 'Testing…' : 'Test' }}</button>
            <button @click.stop="openEdit(a)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" title="Edit"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg></button>
            <button @click.stop="removeAlert(a)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-rose-400" title="Delete"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg></button>
          </div>
        </div>
      </section>
    </div>

    <!-- editor modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="modalOpen = false">
      <div class="w-full max-w-2xl overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <span class="grid h-9 w-9 place-items-center rounded-xl bg-accent/12 text-accent"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg></span>
          <h3 class="text-base font-semibold text-fg">{{ editId ? 'Edit alert rule' : 'New alert rule' }}</h3>
          <button @click="modalOpen = false" class="ml-auto rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" aria-label="Close"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <div class="max-h-[62vh] space-y-6 overflow-auto p-5">
          <!-- 1. source -->
          <div>
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">1</span>What to watch</div>
            <div v-if="!editId" class="mb-2.5 inline-flex overflow-hidden rounded-lg border border-line">
              <button @click="setSrcType('monitor')" class="flex items-center gap-1.5 px-3.5 py-2 text-sm" :class="ed.srcType === 'monitor' ? 'bg-surface2 text-fg' : 'text-muted hover:text-fg'">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>Service
              </button>
              <button @click="setSrcType('host')" class="flex items-center gap-1.5 px-3.5 py-2 text-sm" :class="ed.srcType === 'host' ? 'bg-surface2 text-fg' : 'text-muted hover:text-fg'">
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>Host
              </button>
            </div>
            <select v-if="!editId" v-model="ed.targetId" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
              <option value="">— pick a {{ ed.srcType === 'monitor' ? 'service' : 'host' }} —</option>
              <option v-for="m in (ed.srcType === 'monitor' ? monsInNs : sysInNs)" :key="m.id" :value="m.id">{{ m.name }}</option>
            </select>
            <div v-else class="rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-muted">Source can't be changed — delete and recreate to retarget.</div>
          </div>

          <!-- 2. condition -->
          <div>
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">2</span>When it fires</div>
            <div v-if="ed.srcType === 'monitor'" class="flex items-center gap-2 text-sm text-muted">
              Fires when the service is <span class="rounded-md border border-amber-400/40 bg-amber-400/10 px-2 py-1 font-semibold text-amber-400">DOWN</span>
            </div>
            <div v-else class="flex flex-wrap items-center gap-2.5">
              <span class="text-sm text-muted">Fires when</span>
              <select v-model="ed.condType" class="rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
                <option value="metric">a metric</option><option value="offline">it goes offline</option>
              </select>
              <template v-if="ed.condType === 'metric'">
                <select v-model="ed.metric" class="rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none"><option value="cpu_percent">CPU %</option><option value="mem_percent">Memory %</option><option value="load1">Load 1m</option></select>
                <select v-model="ed.op" class="rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none"><option>&gt;</option><option>&gt;=</option><option>&lt;</option><option>&lt;=</option></select>
                <input v-model.number="ed.value" type="number" class="w-24 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
              </template>
              <template v-else>
                <span class="text-sm text-muted">no sample for</span>
                <input v-model.number="ed.offlineSecs" type="number" class="w-24 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
                <span class="text-sm text-muted">seconds</span>
              </template>
            </div>
          </div>

          <!-- 3. channels -->
          <div>
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">3</span>Notify these channels</div>
            <p v-if="!editChannels.length" class="text-xs text-faint">No channels yet — create one under <b>Alert › Notify channel</b>.</p>
            <div v-else class="flex flex-wrap gap-2">
              <button v-for="c in editChannels" :key="c.id" @click="toggleChan(c.id)"
                class="inline-flex items-center gap-2 rounded-lg border px-3 py-2 text-sm"
                :class="ed.channels.has(c.id) ? 'border-accent/60 bg-accent/8 text-fg' : 'border-line bg-surface2 text-muted hover:text-fg'">
                <span class="grid h-[22px] w-[22px] place-items-center rounded-md" :style="{ background: chanColor(c.kind), color: chanFg(c.kind) }" v-html="iconSvg(chanIcon(c.kind), 13)"></span>
                {{ c.name }}
                <svg v-if="ed.channels.has(c.id)" class="h-4 w-4 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M20 6 9 17l-5-5"/></svg>
              </button>
            </div>
          </div>

          <!-- 4. delivery -->
          <div>
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">4</span>Delivery</div>
            <label class="block max-w-xs">
              <span class="mb-1.5 block text-xs text-faint">Re-notify while still firing</span>
              <select v-model="ed.renotify" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none">
                <option v-for="[v, l] in RENOTIFY" :key="v" :value="v">{{ l }}</option>
              </select>
            </label>
          </div>

          <!-- summary -->
          <div class="flex items-start gap-2.5 rounded-xl border border-accent/30 bg-accent/7 px-3.5 py-3 text-[13px] leading-relaxed">
            <svg class="mt-0.5 h-4 w-4 shrink-0 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m3 21 9-9M15 4V2M20 9h2M17.8 6.2 19 5"/><path d="M9.5 8.5 3 21l12.5-6.5L22 8 16 2z"/></svg>
            <span class="text-muted">When <b class="text-fg">{{ summary.t }}</b> <b class="text-fg">{{ summary.cond }}</b>, notify
              <template v-if="summary.names.length"><b v-for="(n, i) in summary.names" :key="n" class="text-fg">{{ n }}{{ i < summary.names.length - 1 ? ', ' : '' }}</b></template>
              <b v-else class="text-rose-400">no channel yet</b>.
            </span>
          </div>
        </div>

        <div class="flex items-center gap-2.5 border-t border-line bg-surface/60 px-5 py-3.5">
          <button v-if="editId" @click="modalTest" class="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface2 px-3 py-2 text-xs font-medium text-fg hover:border-accent/50">
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/></svg>Send test
          </button>
          <span v-if="err" class="text-xs text-rose-400">{{ err }}</span>
          <span class="ml-auto"></span>
          <button @click="modalOpen = false" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
          <button @click="save" class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">{{ editId ? 'Save changes' : 'Create rule' }}</button>
        </div>
      </div>
    </div>
  </AppShell>
</template>
