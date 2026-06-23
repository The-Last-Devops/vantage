<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const users = ref([])
const namespaces = ref([])
const loading = ref(true)

async function loadUsers() {
  loading.value = true
  try { users.value = await api.get('/api/users') } catch { users.value = [] }
  loading.value = false
}

const SYS = [
  { v: 'user', label: 'User', desc: 'Access only the namespaces granted to them' },
  { v: 'read_all', label: 'Admin · read-only', desc: 'View every namespace, no changes' },
  { v: 'admin', label: 'Admin', desc: 'Full access everywhere' },
]
const sysOf = (u) => (u.is_admin ? 'admin' : u.read_all ? 'read_all' : 'user')
const sysLabel = (u) => SYS.find((r) => r.v === sysOf(u))?.label

const NS_ROLES = [{ v: 'viewer', label: 'Read' }, { v: 'editor', label: 'Write' }, { v: 'owner', label: 'Owner' }]

// ---- add user (plain User; set role in the per-user editor afterwards) ----
const nu = ref({ email: '', password: '' })
const showPw = ref(false)
const adding = ref(false)
const addErr = ref('')
const created = ref(null)
const showCreatedPw = ref(false)

function genPassword() {
  const chars = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
  const a = new Uint32Array(16); crypto.getRandomValues(a)
  nu.value.password = Array.from(a, (n) => chars[n % chars.length]).join('')
  showPw.value = true
}
async function addUser() {
  addErr.value = ''
  if (!nu.value.email.includes('@') || nu.value.password.length < 6) { addErr.value = 'Valid email and a password of 6+ chars.'; return }
  adding.value = true
  try {
    const email = nu.value.email.trim(), password = nu.value.password
    await api.post('/api/users', { email, password })
    created.value = { email, password }; showCreatedPw.value = false
    nu.value = { email: '', password: '' }; showPw.value = false
    await loadUsers()
  } catch (e) { addErr.value = e.status === 409 ? 'A user with that email already exists.' : `Failed (${e.status}).` }
  finally { adding.value = false }
}
const credentialsText = computed(() => created.value ? `Last Monitor\nURL: ${location.origin}\nEmail: ${created.value.email}\nPassword: ${created.value.password}` : '')
function copyCreds(ev) {
  navigator.clipboard?.writeText(credentialsText.value)
  const b = ev.target, o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}

async function removeUser(u) {
  if (!confirm(`Delete user ${u.email}? Their memberships and sessions are removed.`)) return
  try { await api.del(`/api/users/${u.id}`); if (editing.value?.id === u.id) editing.value = null; await loadUsers() }
  catch (e) { alert(e.status === 400 ? "You can't delete yourself." : `Failed (${e.status}).`) }
}

// ---- per-user editor (system role + namespace access) ----
const editing = ref(null)         // the user being edited
const editRole = ref('user')      // system role draft
const editNs = ref({})            // namespace_id -> role ('' = no access)
const editErr = ref('')
const resetPw = ref('')

async function openEdit(u) {
  if (editing.value?.id === u.id) { editing.value = null; return }
  editErr.value = ''; resetPw.value = ''
  editRole.value = sysOf(u)
  const map = {}
  try { for (const m of await api.get(`/api/users/${u.id}/memberships`)) map[m.namespace_id] = m.role } catch {}
  // seed every namespace so the editor lists them all (blank = no access)
  const full = {}; for (const n of namespaces.value) full[n.id] = map[n.id] || ''
  editNs.value = full
  editing.value = u
}

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
  if (resetPw.value.length < 6) { editErr.value = 'Password must be 6+ chars.'; return }
  try { await api.patch(`/api/users/${editing.value.id}`, { password: resetPw.value }); resetPw.value = ''; editErr.value = '✓ Password updated.' }
  catch (e) { editErr.value = `Failed (${e.status}).` }
}

onMounted(async () => {
  if (!isAdmin.value) return
  await loadUsers()
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
})
</script>

<template>
  <AppShell title="Members">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage members.
    </div>
    <div v-else class="mx-auto max-w-4xl space-y-4">
      <!-- add user -->
      <form @submit.prevent="addUser" class="max-w-xl space-y-2">
        <input v-model="nu.email" placeholder="email@company.com" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        <div class="flex items-stretch gap-2">
          <div class="relative flex-1">
            <input v-model="nu.password" :type="showPw ? 'text' : 'password'" placeholder="password" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2 pr-9 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
            <button type="button" @click="showPw = !showPw" :title="showPw ? 'Hide' : 'Show'" class="absolute right-2 top-1/2 -translate-y-1/2 text-muted hover:text-fg">
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path v-if="showPw" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle v-if="showPw" cx="12" cy="12" r="3"/><path v-else d="M3 3l18 18M10.6 10.6a3 3 0 0 0 4.2 4.2M9.9 4.2A10 10 0 0 1 22 12a13 13 0 0 1-2.2 3M6.1 6.1A13 13 0 0 0 2 12s3.5 7 10 7a10 10 0 0 0 3-.5"/></svg>
            </button>
          </div>
          <button type="button" @click="genPassword" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-muted hover:border-accent/50 hover:text-fg">Generate</button>
          <button type="submit" :disabled="adding" class="shrink-0 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90 disabled:opacity-50">{{ adding ? 'Adding…' : 'Add user' }}</button>
        </div>
      </form>
      <p v-if="addErr" class="text-xs text-rose-400">{{ addErr }}</p>

      <!-- credentials hand-off -->
      <div v-if="created" class="max-w-xl rounded-lg border border-accent/40 bg-accent/10 p-3">
        <div class="mb-2 flex items-center justify-between">
          <span class="text-xs font-medium text-accent">User created — send them these credentials</span>
          <div class="flex items-center gap-2">
            <button @click="copyCreds" class="rounded-md border border-line bg-surface2 px-2 py-1 text-xs text-muted hover:text-accent">Copy</button>
            <button @click="created = null" class="text-muted hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
          </div>
        </div>
        <div class="space-y-1 rounded-md bg-bg p-2.5 text-xs leading-relaxed text-fg">
          <div><span class="inline-block w-20 text-faint">Email</span>{{ created.email }}</div>
          <div class="flex items-center gap-2">
            <span><span class="inline-block w-20 text-faint">Password</span><span class="font-mono">{{ showCreatedPw ? created.password : '•'.repeat(created.password.length) }}</span></span>
            <button @click="showCreatedPw = !showCreatedPw" class="text-muted hover:text-accent"><svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path v-if="showCreatedPw" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle v-if="showCreatedPw" cx="12" cy="12" r="3"/><path v-else d="M3 3l18 18M10.6 10.6a3 3 0 0 0 4.2 4.2M9.9 4.2A10 10 0 0 1 22 12a13 13 0 0 1-2.2 3M6.1 6.1A13 13 0 0 0 2 12s3.5 7 10 7a10 10 0 0 0 3-.5"/></svg></button>
          </div>
        </div>
      </div>

      <!-- users list -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">Email</th>
            <th class="px-4 py-3 font-medium">System role</th>
            <th class="px-4 py-3 font-medium text-right">Namespaces</th>
            <th class="px-4 py-3"></th>
          </tr></thead>
          <tbody>
            <tr v-if="loading"><td colspan="4" class="px-4 py-6 text-center text-muted">Loading…</td></tr>
            <template v-for="u in users" :key="u.id">
            <tr class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3 text-fg">{{ u.email }}<span v-if="u.id === auth.user?.id" class="ml-2 text-[10px] uppercase tracking-wider text-faint">you</span></td>
              <td class="px-4 py-3" :class="u.is_admin ? 'text-accent' : 'text-muted'">{{ sysLabel(u) }}</td>
              <td class="px-4 py-3 text-right tabular-nums text-muted">{{ u.namespaces }}</td>
              <td class="px-4 py-3">
                <div class="flex items-center justify-end gap-3">
                  <button @click="openEdit(u)" :class="editing?.id === u.id ? 'text-accent' : 'text-muted hover:text-accent'" title="Edit access">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.1 2.1 0 0 1 3 3L12 15l-4 1 1-4Z"/></svg>
                  </button>
                  <button v-if="u.id !== auth.user?.id" @click="removeUser(u)" title="Delete user" class="text-muted hover:text-rose-400">
                    <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                  </button>
                  <span v-else class="h-4 w-4"></span>
                </div>
              </td>
            </tr>
            <!-- per-user editor -->
            <tr v-if="editing?.id === u.id" class="border-b border-line/60 bg-surface2/40">
              <td colspan="4" class="px-4 py-4">
                <div class="grid gap-5 lg:grid-cols-2">
                  <!-- system role + password -->
                  <div class="space-y-3">
                    <div>
                      <div class="mb-1 text-xs uppercase tracking-wider text-faint">System role</div>
                      <select v-model="editRole" @change="saveSysRole" class="w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none">
                        <option v-for="r in SYS" :key="r.v" :value="r.v">{{ r.label }}</option>
                      </select>
                      <p class="mt-1 text-xs text-faint">{{ SYS.find((r) => r.v === editRole)?.desc }}</p>
                    </div>
                    <div>
                      <div class="mb-1 text-xs uppercase tracking-wider text-faint">Reset password</div>
                      <div class="flex gap-2">
                        <input v-model="resetPw" type="text" placeholder="new password" class="flex-1 rounded-lg border border-line bg-surface px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
                        <button @click="doResetPw" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50">Set</button>
                      </div>
                    </div>
                  </div>
                  <!-- namespace access (hidden for system admins — they see everything) -->
                  <div>
                    <div class="mb-1 text-xs uppercase tracking-wider text-faint">Namespace access</div>
                    <p v-if="editRole !== 'user'" class="text-xs text-muted">{{ editRole === 'admin' ? 'Admins have full access to every namespace.' : 'Read-only admins can view every namespace.' }}</p>
                    <div v-else class="max-h-56 space-y-1 overflow-y-auto pr-1">
                      <div v-for="n in namespaces" :key="n.id" class="flex items-center justify-between gap-2 rounded-lg px-1 py-1">
                        <span class="truncate text-sm text-fg">{{ n.name }}</span>
                        <select :value="editNs[n.id]" @change="setNsRole(n, $event.target.value)" class="shrink-0 rounded-md border border-line bg-surface px-2 py-1 text-xs text-fg focus:border-accent/60 focus:outline-none">
                          <option value="">No access</option>
                          <option v-for="r in NS_ROLES" :key="r.v" :value="r.v">{{ r.label }}</option>
                        </select>
                      </div>
                    </div>
                  </div>
                </div>
                <p v-if="editErr" class="mt-3 text-xs" :class="editErr.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ editErr }}</p>
              </td>
            </tr>
            </template>
          </tbody>
        </table>
      </div>
      <p class="text-xs text-faint">Click the pencil to edit a user. <b>Read</b> = view metrics · <b>Write</b> = add/edit systems &amp; monitors · <b>Owner</b> = also manage members.</p>
    </div>
  </AppShell>
</template>
