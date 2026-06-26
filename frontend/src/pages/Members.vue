<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { minLoad } from '../lib/minLoad'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const users = ref([])
const namespaces = ref([])
const loading = ref(true)

const nameOf = (id) => namespaces.value.find((n) => n.id === id)?.name || id

async function loadUsers() {
  loading.value = true
  try {
    const us = await minLoad(api.get('/api/users'))
    // The list endpoint returns a namespace count only; fetch each member's
    // per-namespace roles so the table can show named chips. Admins see all.
    await Promise.all(
      us.map(async (u) => {
        if (u.is_admin || u.read_all) { u.access = 'all'; return }
        try { u.access = await api.get(`/api/users/${u.id}/memberships`) } catch { u.access = [] }
      }),
    )
    users.value = us
  } catch { users.value = [] }
  loading.value = false
}

// ---- roles ----
const SYS = [
  { v: 'user', label: 'Member', desc: 'Access only the namespaces granted below' },
  { v: 'read_all', label: 'Admin · read-only', desc: 'View every namespace, no changes' },
  { v: 'admin', label: 'Admin', desc: 'Full access everywhere, manages members' },
]
const sysOf = (u) => (u.is_admin ? 'admin' : u.read_all ? 'read_all' : 'user')
const sysLabel = (u) => SYS.find((r) => r.v === sysOf(u))?.label
const NS_ROLES = [{ v: 'viewer', label: 'Viewer' }, { v: 'editor', label: 'Editor' }, { v: 'owner', label: 'Owner' }]
const memberColumns = [
  { key: 'email', label: 'Member', sortable: true, nowrap: false },
  { key: 'sysrole', label: 'System role' },
  { key: 'access', label: 'Namespace access', nowrap: false },
  { key: 'actions', label: '', align: 'right', width: '92px' },
]
const nsRoleLabel = (v) => NS_ROLES.find((r) => r.v === v)?.label || v
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
const nu = ref({ email: '', password: '' })
const showPw = ref(false)
const adding = ref(false)
const addErr = ref('')
const created = ref(null) // { email, password } shown after success
const showCreatedPw = ref(false)

function openAdd() {
  nu.value = { email: '', password: '' }; showPw.value = false
  addErr.value = ''; created.value = null; addOpen.value = true
}
function genPassword(target) {
  const chars = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
  const a = new Uint32Array(16); crypto.getRandomValues(a)
  const pw = Array.from(a, (n) => chars[n % chars.length]).join('')
  if (target === 'reset') resetPw.value = pw
  else { nu.value.password = pw; showPw.value = true }
}
// ASCII email, no whitespace/odd characters — mirrors the server's check.
const EMAIL_RE = /^[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}$/
async function addUser() {
  addErr.value = ''
  const email = nu.value.email.trim()
  if (!EMAIL_RE.test(email)) { addErr.value = 'Enter a valid email — letters, digits and . _ % + - only, no spaces.'; return }
  if (nu.value.password.length < 6) { addErr.value = 'Password must be at least 6 characters.'; return }
  adding.value = true
  try {
    const password = nu.value.password
    await api.post('/api/users', { email, password })
    created.value = { email, password }; showCreatedPw.value = false
    await loadUsers()
  } catch (e) { addErr.value = e.status === 409 ? 'A member with that email already exists.' : `Failed (${e.status}).` }
  finally { adding.value = false }
}
const credentialsText = computed(() => created.value ? `Last Monitor\nURL: ${location.origin}\nEmail: ${created.value.email}\nPassword: ${created.value.password}` : '')
function copyCreds(ev) {
  navigator.clipboard?.writeText(credentialsText.value)
  const b = ev.target, o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}

async function removeUser(u) {
  if (!(await confirm({ title: `Delete ${u.email}?`, message: 'Their namespace access and sessions are removed. This cannot be undone.', danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/users/${u.id}`); if (editing.value?.id === u.id) editing.value = null; await loadUsers() }
  catch (e) { alert(e.status === 400 ? "You can't delete yourself." : `Failed (${e.status}).`) }
}

// ---- edit member (slide-over) ----
const editing = ref(null)
const editRole = ref('user')
const editNs = ref({}) // namespace_id -> role ('' = no access)
const editErr = ref('')
const resetPw = ref('')

async function openEdit(u) {
  editErr.value = ''; resetPw.value = ''
  editRole.value = sysOf(u)
  const map = {}
  try { for (const m of await api.get(`/api/users/${u.id}/memberships`)) map[m.namespace_id] = m.role } catch {}
  const full = {}; for (const n of namespaces.value) full[n.id] = map[n.id] || ''
  editNs.value = full
  editing.value = u
}
function closeEdit() { editing.value = null }

async function saveSysRole() {
  editErr.value = ''
  try { await api.patch(`/api/users/${editing.value.id}`, { is_admin: editRole.value === 'admin', read_all: editRole.value === 'read_all' }); await loadUsers() }
  catch (e) { editErr.value = e.status === 400 ? "You can't change your own admin rights." : `Failed (${e.status}).`; editRole.value = sysOf(editing.value) }
}
async function setNsRole(n, role) {
  editErr.value = ''
  try {
    if (role) await api.post(`/api/namespaces/${n.id}/members`, { email: editing.value.email, role })
    else await api.del(`/api/namespaces/${n.id}/members/${editing.value.id}`)
    editNs.value[n.id] = role
    await loadUsers()
  } catch (e) { editErr.value = `Failed (${e.status}).` }
}
async function doResetPw() {
  editErr.value = ''
  if (resetPw.value.length < 6) { editErr.value = 'Password must be 6+ characters.'; return }
  try { await api.patch(`/api/users/${editing.value.id}`, { password: resetPw.value }); resetPw.value = ''; editErr.value = '✓ Password updated.' }
  catch (e) { editErr.value = `Failed (${e.status}).` }
}

onMounted(async () => {
  if (!isAdmin.value) return
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
  await loadUsers()
})
</script>

<template>
  <AppShell title="Members">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage members.
    </div>
    <div v-else class="space-y-4">
      <p class="max-w-3xl text-xs text-faint">People who can sign in. A member's <b class="text-fg">system role</b> sets platform-wide power; <b class="text-fg">namespace access</b> grants specific namespaces and what they can do inside each.</p>

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
          <span v-if="row.access === 'all'" class="inline-flex rounded-md border border-accent/30 bg-accent/8 px-2 py-0.5 text-xs text-accent">All namespaces</span>
          <span v-else-if="!row.access || !row.access.length" class="inline-flex rounded-md border border-dashed border-line px-2 py-0.5 text-xs text-faint">No namespaces yet</span>
          <div v-else class="flex flex-wrap gap-1.5">
            <span v-for="m in row.access" :key="m.namespace_id" class="inline-flex items-center gap-1 rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-fg">
              {{ nameOf(m.namespace_id) }}<span class="text-faint" :class="{ 'text-accent': m.role === 'owner', 'text-amber-400': m.role === 'editor' }">· {{ nsRoleLabel(m.role) }}</span>
            </span>
          </div>
        </template>
        <template #cell-actions="{ row }">
          <div class="flex items-center justify-end gap-1">
            <button @click.stop="openEdit(row)" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg" v-tip="`Edit`">
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
            </button>
            <button v-if="row.id !== auth.user?.id" @click.stop="removeUser(row)" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-rose-500" v-tip="`Remove`">
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
            <li><b class="text-amber-400">Admin · read-only</b> — views every namespace, no changes.</li>
            <li><b class="text-fg">Member</b> — sees only the namespaces granted to them.</li>
          </ul>
        </div>
        <div class="rounded-xl border border-line bg-surface/50 p-3.5">
          <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-faint">Namespace role</div>
          <ul class="space-y-1.5 text-xs text-muted">
            <li><b class="text-fg">Viewer</b> — view metrics &amp; status.</li>
            <li><b class="text-fg">Editor</b> — also add / edit systems &amp; services.</li>
            <li><b class="text-fg">Owner</b> — also manage that namespace's members.</li>
          </ul>
        </div>
      </div>
    </div>

    <!-- Add member modal -->
    <div v-if="addOpen" class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="addOpen = false">
      <div class="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <h3 class="text-base font-semibold text-fg">{{ created ? 'Member created' : 'Add member' }}</h3>
          <button @click="addOpen = false" class="ml-auto rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <!-- form -->
        <form v-if="!created" @submit.prevent="addUser" class="space-y-4 p-5">
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Email</span>
            <input v-model="nu.email" placeholder="email@company.com" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </label>
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Temporary password</span>
            <div class="flex gap-2">
              <div class="relative flex-1">
                <input v-model="nu.password" :type="showPw ? 'text' : 'password'" placeholder="password" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 pr-9 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                <button type="button" @click="showPw = !showPw" class="absolute right-2 top-1/2 -translate-y-1/2 text-muted hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path v-if="showPw" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle v-if="showPw" cx="12" cy="12" r="3"/><path v-else d="M3 3l18 18M10.6 10.6a3 3 0 0 0 4.2 4.2M9.9 4.2A10 10 0 0 1 22 12a13 13 0 0 1-2.2 3M6.1 6.1A13 13 0 0 0 2 12s3.5 7 10 7a10 10 0 0 0 3-.5"/></svg></button>
              </div>
              <button type="button" @click="genPassword()" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-muted hover:border-accent/50 hover:text-fg">Generate</button>
            </div>
            <span class="mt-1.5 block text-xs text-faint">The new member signs in with this; they can change it later. Set the role after creating.</span>
          </label>
          <p v-if="addErr" class="text-xs text-rose-400">{{ addErr }}</p>
          <div class="flex justify-end gap-2.5 pt-1">
            <button type="button" @click="addOpen = false" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
            <button type="submit" :disabled="adding" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ adding ? 'Adding…' : 'Add member' }}</button>
          </div>
        </form>

        <!-- credentials hand-off -->
        <div v-else class="space-y-3 p-5">
          <p class="text-xs text-muted">Send these credentials to the new member — the password isn't shown again.</p>
          <div class="space-y-1.5 rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">
            <div><span class="inline-block w-20 text-faint">Email</span>{{ created.email }}</div>
            <div class="flex items-center gap-2">
              <span><span class="inline-block w-20 text-faint">Password</span><span class="font-mono">{{ showCreatedPw ? created.password : '•'.repeat(created.password.length) }}</span></span>
              <button @click="showCreatedPw = !showCreatedPw" class="text-muted hover:text-accent"><svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path v-if="showCreatedPw" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle v-if="showCreatedPw" cx="12" cy="12" r="3"/><path v-else d="M3 3l18 18M10.6 10.6a3 3 0 0 0 4.2 4.2M9.9 4.2A10 10 0 0 1 22 12a13 13 0 0 1-2.2 3M6.1 6.1A13 13 0 0 0 2 12s3.5 7 10 7a10 10 0 0 0 3-.5"/></svg></button>
            </div>
          </div>
          <div class="flex justify-end gap-2.5">
            <button @click="copyCreds" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50">Copy</button>
            <button @click="addOpen = false" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Done</button>
          </div>
        </div>
      </div>
    </div>

    <!-- Edit member slide-over -->
    <div v-if="editing" class="fixed inset-0 z-50 flex justify-end bg-black/55 backdrop-blur-sm" @click.self="closeEdit">
      <aside class="flex h-full w-full max-w-[420px] flex-col border-l border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <span class="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-line bg-surface2 text-xs font-semibold text-muted">{{ initials(editing.email) }}</span>
          <span class="min-w-0 flex-1 truncate text-sm font-medium text-fg">{{ editing.email }}</span>
          <button @click="closeEdit" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <div class="flex-1 space-y-6 overflow-y-auto p-5">
          <!-- system role -->
          <div>
            <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">System role</div>
            <UiSelect v-model="editRole" block @update:model-value="saveSysRole" :options="SYS.map((r) => ({ value: r.v, label: r.label }))" />
            <p class="mt-1.5 text-xs text-faint">{{ SYS.find((r) => r.v === editRole)?.desc }}</p>
          </div>

          <!-- namespace access -->
          <div>
            <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Namespace access</div>
            <p v-if="editRole !== 'user'" class="rounded-lg border border-line bg-surface2/40 px-3 py-2.5 text-xs text-muted">{{ editRole === 'admin' ? 'Admins have full access to every namespace.' : 'Read-only admins can view every namespace.' }}</p>
            <div v-else-if="!namespaces.length" class="text-xs text-faint">No namespaces exist yet.</div>
            <div v-else class="divide-y divide-line/60">
              <div v-for="n in namespaces" :key="n.id" class="flex items-center gap-3 py-2.5">
                <span class="flex-1 truncate text-sm" :class="editNs[n.id] ? 'text-fg' : 'text-faint'">{{ n.name }}</span>
                <UiSelect :model-value="editNs[n.id]" @update:model-value="(v) => setNsRole(n, v)" class="shrink-0"
                  :options="[{ value: '', label: '— no access' }, ...NS_ROLES.map((r) => ({ value: r.v, label: r.label }))]" />
              </div>
            </div>
          </div>

          <!-- reset password -->
          <div>
            <div class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-faint">Reset password</div>
            <div class="flex gap-2">
              <input v-model="resetPw" type="text" placeholder="new password" class="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
              <button @click="genPassword('reset')" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-muted hover:border-accent/50 hover:text-fg">Generate</button>
              <button @click="doResetPw" class="shrink-0 rounded-lg bg-accent px-3 py-2.5 text-sm font-semibold text-accentfg hover:opacity-90">Set</button>
            </div>
          </div>

          <p v-if="editErr" class="text-xs" :class="editErr.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ editErr }}</p>
        </div>

        <div class="border-t border-line px-5 py-3.5 text-center">
          <button @click="closeEdit" class="text-sm text-muted hover:text-fg">Changes save as you make them — Close</button>
        </div>
      </aside>
    </div>
  </AppShell>
</template>
