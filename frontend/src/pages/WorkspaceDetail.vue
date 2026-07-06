<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { minLoad } from '../lib/minLoad'
import { useAuth } from '../stores/auth'

const route = useRoute()
const router = useRouter()
const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)
const wsId = computed(() => route.params.id)

const loaded = ref(false)
const ws = ref(null) // { id, name, role, system_count, member_count }
const members = ref([])
const rules = ref([])
const services = ref([])
const keys = ref([])
const thr = ref(null)

const canManage = computed(() => ws.value && (ws.value.role === 'owner' || isAdmin.value))
const canEdit = computed(() => ws.value && (['owner', 'editor', 'admin'].includes(ws.value.role) || isAdmin.value))
const isDefault = computed(() => ws.value?.name === 'default')

const roleClass = (r) => ({ owner: 'text-accent', editor: 'text-warn', viewer: 'text-muted', admin: 'text-accent' }[r] || 'text-muted')
const initials = (e) => (e || '?').slice(0, 2).toUpperCase()

// ---- rule state (mirrors Alerts.vue) ----
function ruleState(a) {
  if (!a.enabled) return { label: 'Disabled', cls: 'text-faint bg-surface2', dot: 'bg-faint' }
  if (a.firing === true) return { label: 'Firing', cls: 'text-down bg-down/12', dot: 'bg-down' }
  if (a.firing === false) return { label: 'OK', cls: 'text-accent bg-accent/12', dot: 'bg-accent' }
  return { label: 'Pending', cls: 'text-warn bg-warn/12', dot: 'bg-warn' }
}
const METRIC = { cpu_percent: 'CPU %', mem_percent: 'Memory %', load1: 'Load 1m' }
function ruleCond(a) {
  const c = a.condition || {}
  if (a.target_kind === 'monitor' || a.target_kind === 'all_services') return 'is DOWN'
  if (c.offline_secs) return `offline > ${c.offline_secs}s`
  if (c.metric) return `${METRIC[c.metric] || c.metric} ${c.op} ${c.value}`
  return '—'
}

// ---- members ----
const WS_ROLES = ['viewer', 'editor', 'owner']
const addEmail = ref('') // selected candidate's email
const addRole = ref('viewer')
const memErr = ref('')
const candidates = ref([]) // existing users not yet in this workspace
async function loadMembers() {
  try { members.value = await api.get(`/api/workspaces/${wsId.value}/members`) } catch { members.value = [] }
}
async function loadCandidates() {
  try { candidates.value = await api.get(`/api/workspaces/${wsId.value}/member-candidates`) } catch { candidates.value = [] }
}
async function addMember() {
  memErr.value = ''
  const email = addEmail.value
  if (!email) { memErr.value = 'Pick a user to add.'; return }
  try {
    await api.post(`/api/workspaces/${wsId.value}/members`, { email, role: addRole.value })
    addEmail.value = ''
    await Promise.all([loadMembers(), loadCandidates()])
  } catch (e) {
    memErr.value = e.status === 403 ? 'You need owner access.' : `Failed (${e.status}).`
  }
}
async function setRole(m, role) {
  memErr.value = ''
  try { await api.post(`/api/workspaces/${wsId.value}/members`, { email: m.email, role }); m.role = role }
  catch (e) { memErr.value = `Failed (${e.status}).`; await loadMembers() }
}
async function removeMember(m) {
  if (m.user_id === auth.user?.id && !(await confirm({ title: 'Remove your own access?', message: `You will lose ${ws.value.role} access to ${ws.value.name}.`, danger: true, confirmText: 'Remove' }))) return
  if (!(await confirm({ title: 'Remove member?', message: `${m.email} will lose access to ${ws.value.name}. You can add them back later.`, danger: true, confirmText: 'Remove' }))) return
  try { await api.del(`/api/workspaces/${wsId.value}/members/${m.user_id}`); await Promise.all([loadMembers(), loadCandidates()]) }
  catch (e) { memErr.value = `Failed (${e.status}).` }
}

// ---- thresholds (Needs attention) ----
const DEFAULT_THR = { cpu_warn: 80, cpu_crit: 90, mem_warn: 80, mem_crit: 90, disk_warn: 80, disk_crit: 90, dutil_warn: 80, dutil_crit: 95 }
const THR_ROWS = [{ key: 'cpu', label: 'CPU' }, { key: 'mem', label: 'Memory' }, { key: 'disk', label: 'Disk space' }, { key: 'dutil', label: 'Disk I/O' }]
const thrForm = ref({ ...DEFAULT_THR })
const thrErr = ref('')
function resetThrForm() {
  const cur = thr.value || DEFAULT_THR
  thrForm.value = Object.fromEntries(Object.keys(DEFAULT_THR).map((k) => [k, cur[k] ?? DEFAULT_THR[k]]))
}
async function saveThr() {
  thrErr.value = ''
  for (const r of THR_ROWS) {
    const w = Number(thrForm.value[r.key + '_warn']), c = Number(thrForm.value[r.key + '_crit'])
    if (!(w >= 0 && c <= 100 && w <= c)) { thrErr.value = `${r.label}: warn ≤ crit, within 0–100.`; return }
  }
  try {
    const body = {}; for (const k in DEFAULT_THR) body[k] = Number(thrForm.value[k])
    await api.put(`/api/workspaces/${wsId.value}/thresholds`, body)
    thr.value = { ...body }; thrErr.value = '✓ Saved.'
  } catch (e) { thrErr.value = e.status === 403 ? 'Editor access required.' : `Failed (${e.status}).` }
}

async function removeWs() {
  if (isDefault.value) return
  if (ws.value.system_count > 0) { alert(`"${ws.value.name}" still has ${ws.value.system_count} system(s). Move or delete them first.`); return }
  if (!(await confirm({ title: 'Delete workspace?', message: `"${ws.value.name}" — this cannot be undone.`, danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/workspaces/${wsId.value}`); router.push({ name: 'workspaces' }) }
  catch (e) { alert(e.status === 409 ? 'Workspace still has systems attached.' : `Failed (${e.status}).`) }
}

onMounted(async () => {
  const work = (async () => {
    const list = await api.get('/api/workspaces').catch(() => [])
    ws.value = list.find((n) => n.id === wsId.value) || null
    if (!ws.value) return
    const [als, mons, ks, thrs] = await Promise.all([
      api.get(`/api/workspaces/${wsId.value}/alerts`).catch(() => []),
      api.get('/api/monitors').catch(() => []),
      api.get(`/api/workspaces/${wsId.value}/keys`).catch(() => []),
      api.get('/api/thresholds').catch(() => []),
    ])
    rules.value = als
    services.value = mons.filter((m) => m.workspace === ws.value.name)
    keys.value = ks
    thr.value = thrs.find((x) => x.workspace === ws.value.name) || null
    resetThrForm()
    if (canManage.value) await Promise.all([loadMembers(), loadCandidates()])
  })()
  await minLoad(work)
  loaded.value = true
})
</script>

<template>
  <AppShell :breadcrumb="[{ label: 'Workspaces', to: { name: 'workspaces' } }, { label: ws?.name || 'Workspace' }]">
    <PageLoader v-if="!loaded" />
    <template v-else-if="!ws">
      <p class="rounded-2xl border border-line bg-surface/50 p-10 text-center text-sm text-muted">Workspace not found, or you don't have access.</p>
    </template>
    <template v-else>
      <div class="mb-5 flex flex-wrap items-center gap-3">
        <span class="h-3 w-3 rounded-full bg-accent"></span>
        <h1 class="text-xl font-bold text-fg">{{ ws.name }}</h1>
        <span v-if="isDefault" class="rounded bg-surface2 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-faint">default</span>
        <span class="rounded-full px-2.5 py-0.5 text-xs font-semibold capitalize" :class="ws.role === 'owner' || ws.role === 'admin' ? 'bg-accent/12 text-accent' : ws.role === 'editor' ? 'bg-warn/12 text-warn' : 'bg-surface2 text-muted'">● {{ ws.role }}</span>
        <button v-if="canManage && !isDefault" @click="removeWs" class="ml-auto rounded-lg border border-down/35 px-3 py-1.5 text-xs font-medium text-down hover:bg-down/10">Delete workspace</button>
      </div>

      <div class="grid items-start gap-4 lg:grid-cols-[1fr_320px]">
        <!-- main -->
        <div class="space-y-4">
          <!-- members -->
          <div class="rounded-2xl border border-line bg-surface p-5">
            <div class="mb-3.5 flex items-center justify-between">
              <h2 class="text-[11px] font-semibold uppercase tracking-wider text-faint">Members · {{ ws.member_count }}</h2>
            </div>
            <template v-if="canManage">
              <div class="mb-3 flex flex-wrap gap-2">
                <UiSelect v-model="addEmail" block class="min-w-[220px] flex-1" :placeholder="candidates.length ? 'Select a user…' : 'All users are already members'" :options="candidates.map((c) => ({ value: c.email, label: c.email }))" />
                <UiSelect v-model="addRole" :options="WS_ROLES.map((r) => ({ value: r, label: r[0].toUpperCase() + r.slice(1) }))" />
                <button @click="addMember" :disabled="!addEmail" class="rounded-lg bg-accent px-3.5 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40">Add</button>
              </div>
              <p v-if="memErr" class="mb-2 text-xs" :class="memErr.startsWith('✓') ? 'text-accent' : 'text-down'">{{ memErr }}</p>
              <div v-if="!members.length" class="text-xs text-faint">No members yet.</div>
              <div v-else class="divide-y divide-line/60">
                <div v-for="m in members" :key="m.user_id" class="flex items-center gap-3 py-2.5">
                  <span class="grid h-8 w-8 shrink-0 place-items-center rounded-lg border text-[11px] font-semibold" :class="m.role === 'owner' ? 'border-accent/25 bg-accent/12 text-accent' : 'border-line bg-surface2 text-muted'">{{ initials(m.email) }}</span>
                  <span class="min-w-0 flex-1 truncate text-sm text-fg">{{ m.email }}<span v-if="m.user_id === auth.user?.id" class="ml-2 rounded border border-accent/40 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-accent">you</span></span>
                  <UiSelect :model-value="m.role" @update:model-value="(v) => setRole(m, v)" class="shrink-0" :options="WS_ROLES.map((r) => ({ value: r, label: r[0].toUpperCase() + r.slice(1) }))" />
                  <button @click="removeMember(m)" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-down" v-tip="`Remove`">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                  </button>
                </div>
              </div>
              <p class="mt-3 text-xs text-faint">Roles apply only inside <b class="text-muted">{{ ws.name }}</b>. System admins always have full access and aren't listed. Pick an existing user, then a role.</p>
            </template>
            <p v-else class="rounded-lg border border-line bg-surface2/40 px-3 py-2.5 text-xs text-muted">Only owners of this workspace (and system admins) can view and manage members. You have <b class="text-fg capitalize">{{ ws.role }}</b> access.</p>
          </div>

          <!-- alert rules -->
          <div class="rounded-2xl border border-line bg-surface p-5">
            <div class="mb-3 flex items-center justify-between">
              <h2 class="text-[11px] font-semibold uppercase tracking-wider text-faint">Alert rules · {{ rules.length }}</h2>
              <RouterLink v-if="canEdit" :to="{ name: 'alert-new', query: { ws: ws.name } }" class="text-xs text-accent hover:underline">New rule ›</RouterLink>
            </div>
            <p v-if="!rules.length" class="text-xs text-faint">No alert rules in this workspace.</p>
            <div v-else>
              <RouterLink v-for="a in rules" :key="a.id" :to="{ name: 'alerts', query: { ws: ws.name, rule: a.id } }" class="flex items-center gap-2.5 border-t border-line/60 py-2.5 first:border-t-0 hover:opacity-80">
                <span class="h-2 w-2 shrink-0 rounded-full" :class="ruleState(a).dot"></span>
                <span class="min-w-0 flex-1 truncate text-sm text-fg"><b>{{ a.target_name }}</b> <span class="text-muted">· {{ ruleCond(a) }}</span></span>
                <span class="shrink-0 rounded-full px-2 py-0.5 text-[11px] font-semibold" :class="ruleState(a).cls">{{ ruleState(a).label }}</span>
              </RouterLink>
            </div>
          </div>
        </div>

        <!-- side rail -->
        <div class="space-y-4">
          <!-- contents -->
          <div class="rounded-2xl border border-line bg-surface p-4">
            <h2 class="mb-3 text-[11px] font-semibold uppercase tracking-wider text-faint">Contents</h2>
            <div class="flex flex-col gap-0.5 text-sm">
              <RouterLink :to="{ name: 'systems', query: { ws: ws.name } }" class="flex items-center justify-between rounded-lg px-2.5 py-2 text-fg hover:bg-surface2"><span>Systems</span><b class="font-mono tabular-nums">{{ ws.system_count }}</b></RouterLink>
              <RouterLink :to="{ name: 'monitors', query: { ws: ws.name } }" class="flex items-center justify-between rounded-lg px-2.5 py-2 text-fg hover:bg-surface2"><span>Services</span><b class="font-mono tabular-nums">{{ services.length }}</b></RouterLink>
              <div class="flex items-center justify-between rounded-lg px-2.5 py-2 text-fg"><span>API keys</span><b class="font-mono tabular-nums">{{ keys.length }}</b></div>
            </div>
          </div>

          <!-- thresholds tucked away -->
          <details class="group overflow-hidden rounded-2xl border border-line bg-surface">
            <summary class="flex cursor-pointer list-none items-center gap-2.5 px-4 py-3.5 text-sm font-semibold text-fg [&::-webkit-details-marker]:hidden">
              <svg class="h-4 w-4 text-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/></svg>
              “Needs attention” thresholds
              <svg class="ml-auto h-4 w-4 text-faint transition-transform group-open:rotate-180" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>
            </summary>
            <div class="border-t border-line px-4 pb-4 pt-3">
              <p class="mb-2 text-xs text-faint">When a host crosses these, it shows under <b class="text-muted">Needs attention</b>.</p>
              <div class="space-y-0.5">
                <div v-for="r in THR_ROWS" :key="r.key" class="flex items-center gap-2 border-t border-line/50 py-2 text-xs first:border-t-0">
                  <span class="flex-1 text-fg">{{ r.label }}</span>
                  <input v-model.number="thrForm[r.key + '_warn']" :disabled="!canEdit" type="number" min="0" max="100" class="w-14 rounded-md border border-line bg-surface2 px-2 py-1 text-center text-warn focus:border-accent/60 focus:outline-none disabled:opacity-60" />
                  <input v-model.number="thrForm[r.key + '_crit']" :disabled="!canEdit" type="number" min="0" max="100" class="w-14 rounded-md border border-line bg-surface2 px-2 py-1 text-center text-down focus:border-accent/60 focus:outline-none disabled:opacity-60" />
                  <span class="text-faint">%</span>
                </div>
              </div>
              <div v-if="canEdit" class="mt-3 flex items-center gap-2.5">
                <button @click="saveThr" class="rounded-lg bg-accent px-3.5 py-1.5 text-sm font-semibold text-accentfg hover:opacity-90">Save</button>
                <button @click="resetThrForm" class="text-xs text-muted hover:text-fg">Reset</button>
                <span v-if="thrErr" class="text-xs" :class="thrErr.startsWith('✓') ? 'text-accent' : 'text-down'">{{ thrErr }}</span>
              </div>
              <p v-else class="mt-2 text-xs text-faint">Editor access required to change these.</p>
            </div>
          </details>
        </div>
      </div>
    </template>
  </AppShell>
</template>
