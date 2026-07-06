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
const monitors = ref([])
const systems = ref([])
const channels = ref([]) // global channel list { id, name, kind, workspace }
const types = ref([]) // channel-type manifest, for provider icon/colors
const loaded = ref(false)
const err = ref('')
const saving = ref(false)

// ---- channel provider icon (kind → color/icon), matches Notify channels ----
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
function iconSvg(name, size = 15) {
  const ic = ICONS[name] || ICONS.chat
  const attrs = ic.fill ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" ${attrs}>${ic.body}</svg>`
}
const chanColor = (kind) => typeByKind(kind)?.color || 'rgb(var(--surface2))'
const chanFg = (kind) => typeByKind(kind)?.fg || 'rgb(var(--fg))'
const chanIcon = (kind) => typeByKind(kind)?.icon || 'chat'

const METRIC_LABEL = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
const ed = ref({ srcType: 'monitor', targetId: '', scopeWs: '', condType: 'down', metric: 'cpu_percent', op: '>', value: 90, offlineSecs: 120, channels: new Set(), renotify: '' })

const isScope = computed(() => ed.value.srcType === 'all_services' || ed.value.srcType === 'all_hosts')
const isServiceLike = computed(() => ed.value.srcType === 'monitor' || ed.value.srcType === 'all_services')
const targetWs = computed(() => {
  const list = ed.value.srcType === 'monitor' ? monitors.value : systems.value
  const name = list.find((x) => x.id === ed.value.targetId)?.workspace
  return workspaces.value.find((n) => n.name === name) || null
})
const saveWs = computed(() => (isScope.value ? workspaces.value.find((n) => n.id === ed.value.scopeWs) || null : targetWs.value))
const candidates = computed(() => (ed.value.srcType === 'all_services' ? monitors.value : systems.value).filter((x) => x.workspace === saveWs.value?.name))

function setSrcType(t) {
  ed.value.srcType = t
  ed.value.targetId = ''
  ed.value.condType = t === 'monitor' || t === 'all_services' ? 'down' : 'metric'
}
function toggleChan(id) {
  const s = ed.value.channels
  s.has(id) ? s.delete(id) : s.add(id)
  ed.value.channels = new Set(s)
}

const targetName = computed(() => {
  if (ed.value.srcType === 'all_services') return 'any service'
  if (ed.value.srcType === 'all_hosts') return 'any host'
  const list = ed.value.srcType === 'monitor' ? monitors.value : systems.value
  return list.find((x) => x.id === ed.value.targetId)?.name || ''
})
const condText = computed(() => {
  if (isServiceLike.value) return 'is DOWN'
  if (ed.value.condType === 'offline') return `offline > ${ed.value.offlineSecs}s`
  return `${METRIC_LABEL[ed.value.metric]} ${ed.value.op} ${ed.value.value}`
})

// ---- per-channel test (works before the rule is saved) ----
const testState = ref({})
async function testChan(id) {
  testState.value = { ...testState.value, [id]: 'run' }
  try { await api.post(`/api/channels/${id}/test`); testState.value = { ...testState.value, [id]: 'ok' } }
  catch { testState.value = { ...testState.value, [id]: 'fail' } }
  setTimeout(() => { testState.value = { ...testState.value, [id]: undefined } }, 3000)
}

function buildCondition() {
  if (isServiceLike.value) return {}
  if (ed.value.condType === 'offline') return { offline_secs: Number(ed.value.offlineSecs) || 120 }
  return { metric: ed.value.metric, op: ed.value.op, value: Number(ed.value.value) }
}
function backToList() { router.push({ name: 'alerts', query: route.query.ws ? { ws: route.query.ws } : {} }) }

// Build the target (source) part of the payload — shared by create and edit.
function targetBody() {
  const b = {}
  if (isScope.value) { b.scope_kind = ed.value.srcType; b.scope_workspace_id = saveWs.value?.id }
  else if (ed.value.srcType === 'monitor') b.monitor_id = ed.value.targetId
  else b.system_id = ed.value.targetId
  return b
}
async function save() {
  err.value = ''
  if (!isScope.value && !ed.value.targetId) { err.value = `Pick a ${ed.value.srcType === 'monitor' ? 'service' : 'host'}.`; return }
  if (!saveWs.value) { err.value = 'Pick a source first.'; return }
  if (!ed.value.channels.size) { err.value = 'Pick at least one channel.'; return }
  const channel_ids = [...ed.value.channels]
  const renotify_secs = ed.value.renotify ? Number(ed.value.renotify) : null
  saving.value = true
  try {
    if (editId.value) {
      // Source is editable now — send the target too (re-targets the rule server-side).
      await api.patch(`/api/alerts/${editId.value}`, { channel_ids, renotify_secs, condition: buildCondition(), ...targetBody() })
    } else {
      await api.post(`/api/workspaces/${saveWs.value.id}/alerts`, { channel_ids, renotify_secs, condition: buildCondition(), ...targetBody() })
    }
    backToList()
  } catch (e) { err.value = e.status === 403 ? 'You need editor access.' : `Failed (${e.status}).` }
  finally { saving.value = false }
}

onMounted(async () => {
  const work = (async () => {
    const [ws, mons, sys, chs, tys] = await Promise.all([
      api.get('/api/workspaces').catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get('/api/systems').catch(() => []),
      api.get('/api/channels').catch(() => []),
      api.get('/api/channel-types').catch(() => []),
    ])
    workspaces.value = ws; monitors.value = mons; systems.value = sys; channels.value = chs; types.value = tys
    if (editId.value) {
      const a = await api.get(`/api/alerts/${editId.value}`)
      const c = a.condition || {}
      const serviceLike = a.scope_kind === 'all_services' || (!a.scope_kind && a.monitor_id)
      ed.value = {
        srcType: a.scope_kind || (a.monitor_id ? 'monitor' : 'host'),
        targetId: a.monitor_id || a.system_id || '',
        scopeWs: a.scope_workspace_id || '',
        condType: serviceLike ? 'down' : c.offline_secs ? 'offline' : 'metric',
        metric: c.metric || 'cpu_percent', op: c.op || '>', value: c.value ?? 90, offlineSecs: c.offline_secs ?? 120,
        channels: new Set((a.channels || []).map((ch) => ch.id)),
        renotify: a.renotify_secs ? String(a.renotify_secs) : '',
      }
    } else {
      ed.value.scopeWs = ws[0]?.id || ''
    }
  })()
  await minLoad(work)
  loaded.value = true
})
</script>

<template>
  <AppShell :breadcrumb="[{ label: 'Rules', to: { name: 'alerts', query: route.query.ws ? { ws: route.query.ws } : {} } }, { label: editId ? 'Edit rule' : 'New rule' }]">
    <PageLoader v-if="!loaded" />
    <template v-else>
      <div class="mx-auto grid w-full max-w-4xl gap-4 lg:grid-cols-[1fr_320px]">
        <!-- form -->
        <div class="overflow-hidden rounded-2xl border border-line bg-surface">
          <!-- 1. source -->
          <div class="border-b border-line p-5">
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">1</span>What to watch</div>
            <div class="mb-2.5 flex flex-wrap overflow-hidden rounded-lg border border-line">
              <button v-for="o in [['monitor','Service'],['all_services','All services'],['host','Host'],['all_hosts','All hosts']]" :key="o[0]"
                @click="setSrcType(o[0])" class="px-3.5 py-2 text-sm" :class="ed.srcType === o[0] ? 'bg-surface2 text-fg' : 'text-muted hover:text-fg'">{{ o[1] }}</button>
            </div>
            <UiSelect v-if="!isScope" v-model="ed.targetId" block
              :placeholder="`— pick a ${ed.srcType === 'monitor' ? 'service' : 'host'} —`"
              :options="(ed.srcType === 'monitor' ? monitors : systems).map((m) => ({ value: m.id, label: `${m.name} · ${m.workspace}` }))" />
            <div v-else>
              <UiSelect v-model="ed.scopeWs" block placeholder="— pick a workspace —" :options="workspaces.map((n) => ({ value: n.id, label: n.name }))" />
              <p class="mt-1.5 text-xs text-faint">Covers every {{ ed.srcType === 'all_services' ? 'service' : 'host' }} in this workspace — new ones included automatically.</p>
            </div>
            <p v-if="editId" class="mt-1.5 text-xs text-faint">Changing the source re-points this rule and resets its current state.</p>
          </div>

          <!-- 2. condition -->
          <div class="border-b border-line p-5">
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">2</span>When it fires</div>
            <div v-if="isServiceLike" class="flex items-center gap-2 text-sm text-muted">
              Fires when {{ ed.srcType === 'all_services' ? 'any service' : 'the service' }} is <span class="rounded-md border border-warn/40 bg-warn/10 px-2 py-1 font-semibold text-warn">DOWN</span>
            </div>
            <div v-else class="flex flex-wrap items-center gap-2.5">
              <span class="text-sm text-muted">Fires when</span>
              <UiSelect v-model="ed.condType" :options="[['metric', 'a metric'], ['offline', 'it goes offline']]" />
              <template v-if="ed.condType === 'metric'">
                <UiSelect v-model="ed.metric" :options="[['cpu_percent', 'CPU %'], ['mem_percent', 'Memory %'], ['load1', 'Load 1m']]" />
                <UiSelect v-model="ed.op" :options="['>', '>=', '<', '<=']" />
                <input v-model.number="ed.value" type="number" class="w-24 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
              </template>
              <template v-else>
                <span class="text-sm text-muted">no sample for</span>
                <input v-model.number="ed.offlineSecs" type="number" class="w-24 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" /><span class="text-sm text-muted">seconds</span>
              </template>
            </div>
          </div>

          <!-- 3. channels (test before save) -->
          <div class="border-b border-line p-5">
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">3</span>Notify these channels — test before you save</div>
            <p v-if="!channels.length" class="text-xs text-faint">No channels yet — create one under <b>Alert › Notify channel</b>.</p>
            <div v-else class="space-y-2">
              <div v-for="c in channels" :key="c.id" class="flex items-center gap-2 rounded-lg border px-3 py-2"
                :class="ed.channels.has(c.id) ? 'border-accent/60 bg-accent/8' : 'border-line bg-surface2'">
                <button @click="toggleChan(c.id)" class="flex min-w-0 flex-1 items-center gap-2 text-left">
                  <svg v-if="ed.channels.has(c.id)" class="h-4 w-4 shrink-0 text-accent" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M20 6 9 17l-5-5"/></svg>
                  <span v-else class="h-4 w-4 shrink-0 rounded border border-line"></span>
                  <span class="grid h-6 w-6 shrink-0 place-items-center rounded-md" :style="{ background: chanColor(c.kind), color: chanFg(c.kind) }" v-html="iconSvg(chanIcon(c.kind), 14)"></span>
                  <span class="truncate text-sm text-fg">{{ c.name }}</span>
                  <span class="shrink-0 text-[11px] text-faint">{{ c.kind }} · {{ c.workspace }}</span>
                </button>
                <span v-if="testState[c.id] === 'ok'" class="text-xs text-accent">✓ sent</span>
                <span v-else-if="testState[c.id] === 'fail'" class="text-xs text-down">✗ failed</span>
                <button @click="testChan(c.id)" :disabled="testState[c.id] === 'run'" class="shrink-0 rounded-lg border border-line bg-surface px-2.5 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50">{{ testState[c.id] === 'run' ? 'Testing…' : 'Send test' }}</button>
              </div>
            </div>
          </div>

          <!-- 4. delivery -->
          <div class="p-5">
            <div class="mb-2.5 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-faint"><span class="grid h-[18px] w-[18px] place-items-center rounded bg-surface2 text-accent">4</span>Delivery</div>
            <label class="block max-w-xs"><span class="mb-1.5 block text-xs text-faint">Re-notify while still firing</span>
              <UiSelect v-model="ed.renotify" block :options="[['', 'Off — notify once'], ['900', 'every 15 min'], ['1800', 'every 30 min'], ['3600', 'every hour']]" />
            </label>
          </div>

          <div class="flex items-center gap-2.5 border-t border-line bg-surface/60 px-5 py-3.5">
            <span v-if="err" class="text-xs text-down">{{ err }}</span>
            <span class="ml-auto"></span>
            <button @click="backToList" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
            <button @click="save" :disabled="saving" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ saving ? 'Saving…' : 'Save rule' }}</button>
          </div>
        </div>

        <!-- right rail: wiring -->
        <div class="rounded-2xl border border-line bg-surface p-4">
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Wiring</div>
          <p class="text-[13px] leading-relaxed text-muted">When <b class="text-fg">{{ targetName || '<source>' }}</b><template v-if="isScope && saveWs"> in <b class="text-fg">{{ saveWs.name }}</b></template> <b class="text-fg">{{ condText }}</b>, notify
            <template v-if="ed.channels.size"><b v-for="(id, i) in [...ed.channels]" :key="id" class="text-fg">{{ channels.find((c) => c.id === id)?.name }}{{ i < ed.channels.size - 1 ? ', ' : '' }}</b></template>
            <b v-else class="text-down">no channel yet</b>.
          </p>
          <template v-if="isScope && !editId">
            <div class="mb-2 mt-4 text-[11px] font-semibold uppercase tracking-wide text-faint">Covers {{ candidates.length }} {{ ed.srcType === 'all_services' ? 'services' : 'hosts' }}</div>
            <div class="max-h-64 space-y-1 overflow-y-auto">
              <div v-for="t in candidates" :key="t.id" class="truncate rounded-md bg-surface2 px-2 py-1 text-xs text-fg">{{ t.name }}</div>
              <p v-if="!candidates.length" class="text-xs text-faint">No {{ ed.srcType === 'all_services' ? 'services' : 'hosts' }} in this workspace yet.</p>
            </div>
          </template>
        </div>
      </div>
    </template>
  </AppShell>
</template>
