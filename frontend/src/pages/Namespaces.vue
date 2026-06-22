<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const router = useRouter()

const rows = ref([])
const loading = ref(true)
const err = ref('')
const newName = ref('')
const creating = ref(false)

async function load() {
  loading.value = true
  try { rows.value = await api.get('/api/namespaces') } catch { rows.value = [] }
  loading.value = false
}
onMounted(load)

// k8s-style DNS label, mirrors the server-side validator.
function valid(name) {
  return /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name)
}

async function create() {
  err.value = ''
  const name = newName.value.trim()
  if (!valid(name)) { err.value = 'Lowercase letters, digits and hyphens; must start/end alphanumeric (max 63).'; return }
  creating.value = true
  try {
    await api.post('/api/namespaces', { name })
    newName.value = ''
    await load()
  } catch (e) {
    err.value = e.status === 500 ? 'A namespace with that name already exists.' : `Failed (${e.status}).`
  } finally { creating.value = false }
}

async function remove(ns) {
  if (ns.name === 'default') return
  if (ns.system_count > 0) { alert(`"${ns.name}" still has ${ns.system_count} system(s). Move or delete them first.`); return }
  if (!confirm(`Delete namespace "${ns.name}"? This cannot be undone.`)) return
  try { await api.del(`/api/namespaces/${ns.id}`); await load() }
  catch (e) {
    if (e.status === 409) alert('Namespace still has systems attached.')
    else if (e.status === 403) alert('You cannot delete this namespace.')
    else alert(`Failed (${e.status}).`)
  }
}

function viewSystems(ns) {
  router.push({ name: 'systems', query: { ns: ns.name } })
}

const roleClass = (r) => ({
  owner: 'text-accent',
  editor: 'text-fg',
  viewer: 'text-muted',
  admin: 'text-accent',
}[r] || 'text-muted')
</script>

<template>
  <AppShell title="Namespaces">
    <div class="mx-auto max-w-4xl space-y-6">
      <!-- create -->
      <form @submit.prevent="create" class="flex flex-wrap items-start gap-2">
        <div class="flex-1 min-w-56">
          <input v-model="newName" placeholder="new-namespace"
            class="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          <p v-if="err" class="mt-1 text-xs text-rose-400">{{ err }}</p>
        </div>
        <button type="submit" :disabled="creating"
          class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-black hover:opacity-90 disabled:opacity-50">
          {{ creating ? 'Creating…' : 'Create namespace' }}
        </button>
      </form>

      <!-- list -->
      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
              <th class="px-4 py-3 font-medium">Name</th>
              <th class="px-4 py-3 font-medium">Your role</th>
              <th class="px-4 py-3 font-medium text-right">Systems</th>
              <th class="px-4 py-3 font-medium text-right">Members</th>
              <th class="px-4 py-3"></th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="loading"><td colspan="5" class="px-4 py-6 text-center text-muted">Loading…</td></tr>
            <tr v-else-if="!rows.length"><td colspan="5" class="px-4 py-6 text-center text-muted">No namespaces yet.</td></tr>
            <tr v-for="ns in rows" :key="ns.id" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3">
                <span class="font-medium text-fg">{{ ns.name }}</span>
                <span v-if="ns.name === 'default'" class="ml-2 rounded bg-surface2 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-faint">default</span>
              </td>
              <td class="px-4 py-3 capitalize" :class="roleClass(ns.role)">{{ ns.role }}</td>
              <td class="px-4 py-3 text-right tabular-nums">
                <button @click="viewSystems(ns)" class="text-fg hover:text-accent">{{ ns.system_count }}</button>
              </td>
              <td class="px-4 py-3 text-right tabular-nums text-muted">{{ ns.member_count }}</td>
              <td class="px-4 py-3 text-right">
                <button v-if="ns.name !== 'default'" @click="remove(ns)"
                  :title="ns.system_count > 0 ? 'Has systems — cannot delete' : 'Delete namespace'"
                  class="text-muted hover:text-rose-400 disabled:opacity-30"
                  :disabled="ns.system_count > 0">
                  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="text-xs text-faint">Namespaces group systems and scope access. The <code class="text-muted">default</code> namespace can't be deleted; others must be empty first.</p>
    </div>
  </AppShell>
</template>
