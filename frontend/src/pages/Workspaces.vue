<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'

const router = useRouter()
const rows = ref([])
const err = ref('')
const newName = ref('')
const creating = ref(false)

const { loaded, reload: load } = useCached({
  key: () => 'workspaces',
  load: async () => {
    try { return await api.get('/api/workspaces') } catch { return [] }
  },
  apply: (d) => { rows.value = d },
})
onMounted(load)

// k8s-style DNS label, mirrors the server-side validator.
const valid = (name) => /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name)

async function create() {
  err.value = ''
  const name = newName.value.trim()
  if (!valid(name)) { err.value = 'Lowercase letters, digits and hyphens; must start/end alphanumeric (max 63).'; return }
  creating.value = true
  try { await api.post('/api/workspaces', { name }); newName.value = ''; await load() }
  catch (e) { err.value = e.status === 500 ? 'A workspace with that name already exists.' : `Failed (${e.status}).` }
  finally { creating.value = false }
}

const open = (ws) => router.push({ name: 'workspace', params: { id: ws.id } })
const rolePill = (r) => (r === 'owner' || r === 'admin' ? 'bg-accent/12 text-accent' : r === 'editor' ? 'bg-warn/12 text-warn' : 'bg-surface2 text-muted')
</script>

<template>
  <AppShell title="Workspaces">
    <div class="space-y-5">
      <!-- create -->
      <form @submit.prevent="create" class="flex flex-wrap items-start gap-2">
        <p class="mr-auto max-w-xl text-xs text-faint">Workspaces group systems &amp; services and scope who can see them. Click one to manage members, alert rules and thresholds.</p>
        <div>
          <input v-model="newName" placeholder="new-workspace" class="w-48 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          <p v-if="err" class="mt-1 text-xs text-down">{{ err }}</p>
        </div>
        <button type="submit" :disabled="creating" class="inline-flex items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M12 5v14M5 12h14"/></svg>{{ creating ? 'Creating…' : 'Create' }}
        </button>
      </form>

      <PageLoader v-if="!loaded" />
      <template v-else>
        <p v-if="!rows.length" class="rounded-2xl border border-line bg-surface/50 p-10 text-center text-sm text-muted">No workspaces yet.</p>
        <div v-else class="grid gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
          <button v-for="ws in rows" :key="ws.id" @click="open(ws)"
            class="flex flex-col gap-3.5 rounded-2xl border border-line bg-surface p-4 text-left transition-colors hover:border-accent/55">
            <div class="flex items-center gap-2.5">
              <span class="h-2.5 w-2.5 shrink-0 rounded-full bg-accent"></span>
              <span class="truncate text-base font-semibold text-fg">{{ ws.name }}</span>
              <span v-if="ws.name === 'default'" class="rounded bg-surface2 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-faint">default</span>
              <span class="ml-auto shrink-0 rounded-full px-2.5 py-0.5 text-[11px] font-semibold capitalize" :class="rolePill(ws.role)">● {{ ws.role }}</span>
            </div>
            <div class="flex gap-8">
              <div><div class="text-xl font-bold font-mono tabular-nums text-fg">{{ ws.system_count }}</div><div class="text-[10px] uppercase tracking-wide text-faint">Systems</div></div>
              <div><div class="text-xl font-bold font-mono tabular-nums text-fg">{{ ws.member_count }}</div><div class="text-[10px] uppercase tracking-wide text-faint">Members</div></div>
            </div>
          </button>
        </div>
      </template>
    </div>
  </AppShell>
</template>
