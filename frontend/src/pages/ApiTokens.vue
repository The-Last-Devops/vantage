<script setup>
import { ref, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { useCached } from '../lib/cache'

const tokens = ref([])

const { loaded, reload: load } = useCached({
  key: () => 'api-tokens',
  load: async () => ({ tokens: await api.get('/api/pats') }),
  apply: (d) => { tokens.value = d.tokens },
  onError: () => { tokens.value = [] },
})

// ---- create ----
const modalOpen = ref(false)
const form = ref({ name: '', expires: '' }) // expires: '' = never, else days
const creating = ref(false)
const err = ref('')
const created = ref(null) // { token } shown once
const copied = ref(false)
const EXPIRES = [['', 'Never'], ['30', '30 days'], ['90', '90 days'], ['365', '1 year']]

function openNew() {
  form.value = { name: '', expires: '' }; err.value = ''; created.value = null
  copied.value = false; modalOpen.value = true
}
async function create() {
  err.value = ''
  if (!form.value.name.trim()) { err.value = 'Give the token a name.'; return }
  creating.value = true
  try {
    const body = { name: form.value.name.trim() }
    if (form.value.expires) body.expires_in_days = Number(form.value.expires)
    created.value = await api.post('/api/pats', body)
    await load()
  } catch (e) { err.value = `Failed (${e.status}).` }
  finally { creating.value = false }
}
function copyToken() {
  navigator.clipboard?.writeText(created.value.token)
  copied.value = true; setTimeout(() => (copied.value = false), 1500)
}
async function revoke(t) {
  if (!(await confirm({ title: `Revoke "${t.name}"?`, message: 'Anything using this token stops working immediately. This cannot be undone.', danger: true, confirmText: 'Revoke' }))) return
  try { await api.del(`/api/pats/${t.id}`); await load() } catch (e) { alert(`Failed (${e.status}).`) }
}

const fmt = (s) => (s ? new Date(s).toLocaleString([], { month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }) : '—')
const expired = (t) => t.expires_at && new Date(t.expires_at) < new Date()

onMounted(load)
</script>

<template>
  <AppShell title="API tokens">
    <div class="space-y-4">
      <div class="flex items-start gap-3">
        <p class="max-w-3xl text-xs text-faint">Personal access tokens for the API and the MCP server. A token acts as <b class="text-fg">you</b> — it can do whatever your account can, in the namespaces you belong to. Send it as <code class="rounded bg-surface2 px-1">Authorization: Bearer &lt;token&gt;</code>. The secret is shown once.</p>
        <button @click="openNew" class="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M12 5v14M5 12h14"/></svg>New token
        </button>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <table class="w-full text-sm">
          <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
            <th class="px-4 py-3 font-medium">Name</th>
            <th class="px-4 py-3 font-medium">Token</th>
            <th class="px-4 py-3 font-medium">Created</th>
            <th class="px-4 py-3 font-medium">Last used</th>
            <th class="px-4 py-3 font-medium">Expires</th>
            <th class="px-4 py-3"></th>
          </tr></thead>
          <tbody>
            <tr v-if="!loaded"><td colspan="6"><PageLoader min-height="40vh" /></td></tr>
            <tr v-else-if="!tokens.length"><td colspan="6" class="px-4 py-10 text-center text-muted">No tokens yet. Create one to use the API or connect an MCP client.</td></tr>
            <tr v-for="t in tokens" :key="t.id" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
              <td class="px-4 py-3 text-fg">{{ t.name }}</td>
              <td class="px-4 py-3 font-mono text-xs text-muted">{{ t.prefix }}</td>
              <td class="px-4 py-3 tabular-nums text-muted">{{ fmt(t.created_at) }}</td>
              <td class="px-4 py-3 tabular-nums text-muted">{{ t.last_used ? fmt(t.last_used) : 'never' }}</td>
              <td class="px-4 py-3 tabular-nums" :class="expired(t) ? 'text-rose-400' : 'text-muted'">{{ t.expires_at ? fmt(t.expires_at) + (expired(t) ? ' (expired)' : '') : 'never' }}</td>
              <td class="px-4 py-3 text-right">
                <button @click="revoke(t)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-rose-400" v-tip="`Revoke`">
                  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- create modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="modalOpen = false">
      <div class="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <h3 class="text-base font-semibold text-fg">{{ created ? 'Token created' : 'New API token' }}</h3>
          <button @click="modalOpen = false" class="ml-auto rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" aria-label="Close"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <form v-if="!created" @submit.prevent="create" class="space-y-4 p-5">
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Name</span>
            <input v-model="form.name" placeholder="e.g. ci-pipeline, claude-mcp" class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </label>
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Expires</span>
            <UiSelect v-model="form.expires" block :options="EXPIRES" />
          </label>
          <p v-if="err" class="text-xs text-rose-400">{{ err }}</p>
          <div class="flex justify-end gap-2.5 pt-1">
            <button type="button" @click="modalOpen = false" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
            <button type="submit" :disabled="creating" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ creating ? 'Creating…' : 'Create token' }}</button>
          </div>
        </form>

        <div v-else class="space-y-3 p-5">
          <p class="text-xs text-muted">Copy this token now — it won't be shown again.</p>
          <div class="flex items-center gap-2 rounded-lg border border-line bg-bg p-3">
            <code class="min-w-0 flex-1 break-all font-mono text-xs text-accent">{{ created.token }}</code>
            <button @click="copyToken" class="shrink-0 rounded-lg border border-line bg-surface2 px-2.5 py-1.5 text-xs text-fg hover:border-accent/50">{{ copied ? 'Copied' : 'Copy' }}</button>
          </div>
          <div class="flex justify-end">
            <button @click="modalOpen = false" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Done</button>
          </div>
        </div>
      </div>
    </div>
  </AppShell>
</template>
