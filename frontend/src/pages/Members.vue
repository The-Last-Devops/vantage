<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import MemberAddForm from '../components/MemberAddForm.vue'
import MemberRoleEditor from '../components/MemberRoleEditor.vue'
import MemberAccessChips from '../components/MemberAccessChips.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { useCached } from '../lib/cache'
import { passwordProblem, passwordOk } from '../lib/password'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const users = ref([])
const workspaces = ref([])

const nameOf = (id) => workspaces.value.find((n) => n.id === id)?.name || id

const { loaded: usersLoaded, reload: loadUsers } = useCached({
  key: () => 'members:users',
  load: async () => {
    const us = await api.get('/api/users')
    // The list endpoint returns a workspace count only; fetch each member's
    // per-workspace roles so the table can show named chips. Admins see all.
    await Promise.all(
      us.map(async (u) => {
        if (u.is_admin || u.read_all) { u.access = 'all'; return }
        try { u.access = await api.get(`/api/users/${u.id}/memberships`) } catch { u.access = [] }
      }),
    )
    return us
  },
  apply: (us) => { users.value = us },
  onError: () => { users.value = [] },
})
const loading = computed(() => !usersLoaded.value)

// ---- roles ----
const SYS = [
  { v: 'user', label: 'Member', desc: 'Access only the workspaces granted below' },
  { v: 'read_all', label: 'Admin · read-only', desc: 'View every workspace, no changes' },
  { v: 'admin', label: 'Admin', desc: 'Full access everywhere, manages members' },
]
const sysOf = (u) => (u.is_admin ? 'admin' : u.read_all ? 'read_all' : 'user')
const sysLabel = (u) => SYS.find((r) => r.v === sysOf(u))?.label
const WS_ROLES = [{ v: 'viewer', label: 'Viewer' }, { v: 'editor', label: 'Editor' }, { v: 'owner', label: 'Owner' }]
const memberColumns = [
  { key: 'email', label: 'Member', sortable: true, nowrap: false },
  { key: 'sysrole', label: 'System role' },
  { key: 'access', label: 'Workspace access', nowrap: false },
  { key: 'actions', label: '', align: 'right', width: '92px' },
]
const wsRoleLabel = (v) => WS_ROLES.find((r) => r.v === v)?.label || v
const initials = (email) => (email || '?').slice(0, 2).toUpperCase()

// ---- search + filter ----
const q = ref('')
const filter = ref('all') // all | admin | member
const shown = computed(() => {
  const needle = q.value.toLowerCase().trim()
  return users.value.filter((u) => {
    if (needle && !u.email.toLowerCase().includes(needle)) return false
    if (filter.value === 'admin') return u.is_admin || u.read_all
    if (filter.value === 'member') return !u.is_admin && !u.read_all
    return true
  })
})

// ---- add member (modal) ----
const addOpen = ref(false)
const adding = ref(false)
const addErr = ref('')
const created = ref(null) // { email, password } shown after success

function openAdd() {
  addErr.value = ''; created.value = null; addOpen.value = true
}
function genResetPw() {
  const chars = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*-_=+'
  do {
    const a = new Uint32Array(18); crypto.getRandomValues(a)
    resetPw.value = Array.from(a, (n) => chars[n % chars.length]).join('')
  } while (!passwordOk(resetPw.value))
}
// ASCII email, no whitespace/odd characters — mirrors the server's check.
const EMAIL_RE = /^[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}$/
async function addUser({ email: rawEmail, password }) {
  addErr.value = ''
  const email = rawEmail.trim()
  if (!EMAIL_RE.test(email)) { addErr.value = 'Enter a valid email — letters, digits and . _ % + - only, no spaces.'; return }
  if (password.length < 6) { addErr.value = 'Password must be at least 6 characters.'; return }
  adding.value = true
  try {
    await api.post('/api/users', { email, password })
    created.value = { email, password }
    await loadUsers()
  } catch (e) { addErr.value = e.status === 409 ? 'A member with that email already exists.' : `Failed (${e.status}).` }
  finally { adding.value = false }
}

async function removeUser(u) {
  if (!(await confirm({ title: `Delete ${u.email}?`, message: 'Their workspace access and sessions are removed. This cannot be undone.', danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/users/${u.id}`); if (editing.value?.id === u.id) editing.value = null; await loadUsers() }
  catch (e) { alert(e.status === 400 ? "You can't delete yourself." : `Failed (${e.status}).`) }
}

// ---- edit member (slide-over) ----
const editing = ref(null)
const editRole = ref('user')
const editWs = ref({}) // workspace_id -> role ('' = no access)
const editWsExec = ref({}) // workspace_id -> can_exec (shell access)
const editErr = ref('')
const resetPw = ref('')

// The dialog edits a DRAFT. `orig*` is the snapshot it was opened with, so `dirty`
// is a real diff and Save only sends what actually changed. (It used to PATCH on every
// dropdown change — a mis-click was live before you noticed it, with no way back.)
const origRole = ref('user')
const origWs = ref({})
const origWsExec = ref({})
const savingEdit = ref(false)

const dirty = computed(() => {
  if (!editing.value) return false
  if (editRole.value !== origRole.value) return true
  for (const n of workspaces.value) {
    if ((editWs.value[n.id] || '') !== (origWs.value[n.id] || '')) return true
    if (!!editWsExec.value[n.id] !== !!origWsExec.value[n.id]) return true
  }
  return false
})

async function openEdit(u) {
  editErr.value = ''; resetPw.value = ''
  editRole.value = sysOf(u)
  const map = {}, execMap = {}
  try {
    for (const m of await api.get(`/api/users/${u.id}/memberships`)) {
      map[m.workspace_id] = m.role
      execMap[m.workspace_id] = !!m.can_exec
    }
  } catch {}
  const full = {}; for (const n of workspaces.value) full[n.id] = map[n.id] || ''
  editWs.value = full
  editWsExec.value = execMap
  origRole.value = editRole.value
  origWs.value = { ...full }
  origWsExec.value = { ...execMap }
  editing.value = u
}
async function closeEdit() {
  if (dirty.value && !(await confirm({ title: 'Discard changes?', message: 'The changes to this member have not been saved.', danger: true, confirmText: 'Discard' }))) return
  editing.value = null
}

// Draft mutations — local only, nothing leaves the browser until saveEdit().
function setWsRole(n, role) {
  editWs.value = { ...editWs.value, [n.id]: role }
  if (!role) editWsExec.value = { ...editWsExec.value, [n.id]: false } // no membership, no exec
}
function setWsExec(n, val) {
  editWsExec.value = { ...editWsExec.value, [n.id]: val }
}

async function saveEdit() {
  editErr.value = ''; savingEdit.value = true
  const u = editing.value
  try {
    if (editRole.value !== origRole.value) {
      await api.patch(`/api/users/${u.id}`, { is_admin: editRole.value === 'admin', read_all: editRole.value === 'read_all' })
    }
    for (const n of workspaces.value) {
      const role = editWs.value[n.id] || ''
      const was = origWs.value[n.id] || ''
      if (role !== was) {
        if (role) await api.post(`/api/workspaces/${n.id}/members`, { email: u.email, role })
        else await api.del(`/api/workspaces/${n.id}/members/${u.id}`)
      }
      // Exec only means anything while a membership exists; skip it when access was dropped.
      const ex = !!editWsExec.value[n.id]
      if (role && ex !== !!origWsExec.value[n.id]) {
        await api.put(`/api/workspaces/${n.id}/members/${u.id}/exec`, { can_exec: ex })
      }
    }
    await loadUsers()
    editing.value = null
  } catch (e) {
    const msg = e.status === 400 ? "You can't change your own admin rights." : `Failed (${e.status}).`
    // Re-read from the server so the dialog never shows a half-applied state — some of
    // the calls above may already have landed. openEdit() clears editErr, so set the
    // message after it.
    await loadUsers()
    const still = users.value.find((x) => x.id === u.id)
    if (still) await openEdit(still)
    editErr.value = msg
  } finally { savingEdit.value = false }
}
async function doResetPw() {
  editErr.value = ''
  const problem = passwordProblem(resetPw.value)
  if (problem) { editErr.value = problem; return }
  try { await api.patch(`/api/users/${editing.value.id}`, { password: resetPw.value }); resetPw.value = ''; editErr.value = '✓ Password updated.' }
  catch (e) { editErr.value = `Failed (${e.status}).` }
}

onMounted(async () => {
  if (!isAdmin.value) return
  try { workspaces.value = await api.get('/api/workspaces') } catch { workspaces.value = [] }
  await loadUsers()
})
</script>

<template>
  <AppShell title="Members">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage members.
    </div>
    <div v-else class="space-y-4">
      <p class="max-w-3xl text-xs text-faint">People who can sign in. A member's <b class="text-fg">system role</b> sets platform-wide power; <b class="text-fg">workspace access</b> grants specific workspaces and what they can do inside each.</p>

      <DataTable :columns="memberColumns" :rows="shown" :row-key="(r) => r.id" :loading="loading" :filterable="false"
        clickable @row-click="openEdit" empty="No members yet." empty-filtered="No members match.">
        <template #toolbar>
          <div class="relative min-w-[220px] flex-1">
            <svg class="absolute left-3 top-2.5 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
            <input v-model="q" placeholder="Search members by email…" class="w-full rounded-lg border border-line bg-surface2 py-1.5 pl-9 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </div>
          <div class="inline-flex overflow-hidden rounded-lg border border-line">
            <button v-for="f in [{ v: 'all', l: 'All' }, { v: 'admin', l: 'Admins' }, { v: 'member', l: 'Members' }]" :key="f.v"
              @click="filter = f.v" class="px-3 py-1.5 text-sm" :class="filter === f.v ? 'bg-accent/12 text-accent' : 'text-muted hover:text-fg'">{{ f.l }}</button>
          </div>
          <button @click="openAdd" class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-3.5 py-1.5 text-sm font-semibold text-accentfg hover:opacity-90">
            <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M12 5v14M5 12h14"/></svg>Add member
          </button>
        </template>

        <template #cell-email="{ row }">
          <div class="flex items-center gap-3">
            <span class="grid h-8 w-8 shrink-0 place-items-center rounded-lg border text-[11px] font-semibold"
              :class="(row.is_admin || row.read_all) ? 'border-accent/25 bg-accent/12 text-accent' : 'border-line bg-surface2 text-muted'">{{ initials(row.email) }}</span>
            <span class="font-medium text-fg">{{ row.email }}<span v-if="row.id === auth.user?.id" class="ml-2 rounded border border-accent/40 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-accent">you</span></span>
          </div>
        </template>
        <template #cell-sysrole="{ row }">
          <StatePill :tone="row.is_admin ? 'info' : row.read_all ? 'warn' : 'muted'" :label="sysLabel(row)" />
        </template>
        <template #cell-access="{ row }">
          <MemberAccessChips :access="row.access" :name-of="nameOf" :ws-role-label="wsRoleLabel" />
        </template>
        <template #cell-actions="{ row }">
          <div class="flex items-center justify-end gap-1">
            <button data-t="edit" @click.stop="openEdit(row)" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="`Edit`">
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
            </button>
            <button v-if="row.id !== auth.user?.id" @click.stop="removeUser(row)" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-down" v-tip="`Remove`">
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
            </button>
            <span v-else class="h-8 w-8"></span>
          </div>
        </template>
      </DataTable>

      <!-- role legend -->
      <div class="grid max-w-3xl gap-4 sm:grid-cols-2">
        <div class="rounded-xl border border-line bg-surface/50 p-3.5">
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">System role</div>
          <ul class="space-y-1.5 text-xs text-muted">
            <li><b class="text-accent">Admin</b> — full access to everything, manages members.</li>
            <li><b class="text-warn">Admin · read-only</b> — views every workspace, no changes.</li>
            <li><b class="text-fg">Member</b> — sees only the workspaces granted to them.</li>
          </ul>
        </div>
        <div class="rounded-xl border border-line bg-surface/50 p-3.5">
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">Workspace role</div>
          <ul class="space-y-1.5 text-xs text-muted">
            <li><b class="text-fg">Viewer</b> — view metrics &amp; status.</li>
            <li><b class="text-fg">Editor</b> — also add / edit systems &amp; services.</li>
            <li><b class="text-fg">Owner</b> — also manage that workspace's members.</li>
          </ul>
        </div>
      </div>
    </div>

    <!-- Add member modal -->
    <MemberAddForm v-if="addOpen" :adding="adding" :error="addErr" :created="created"
      @submit="addUser" @close="addOpen = false" />

    <!-- Edit member slide-over -->
    <MemberRoleEditor v-if="editing" :member="editing" :sys="SYS" :ws-roles="WS_ROLES"
      :workspaces="workspaces" :edit-ws="editWs" :edit-ws-exec="editWsExec" :error="editErr" :initials="initials"
      v-model:edit-role="editRole" v-model:reset-pw="resetPw" :dirty="dirty" :saving="savingEdit"
      @close="closeEdit" @save="saveEdit" @set-ws-role="setWsRole" @set-ws-exec="setWsExec"
      @gen-password="genResetPw" @reset-password="doResetPw" />
  </AppShell>
</template>
