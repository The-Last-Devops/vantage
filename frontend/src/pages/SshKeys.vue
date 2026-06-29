<script setup>
// The caller's account-level SSH key library. Every user manages their own keys
// here (account-scoped, not per-system); a key is sealed with the user's account
// password on save and can only be unsealed by them at connect time. The API
// redacts the private key on read — we only ever show name + fingerprint.
import { ref, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { confirm } from '../lib/confirm'
import { useCached } from '../lib/cache'

const keys = ref([])

const { loaded, reload: load } = useCached({
  key: () => 'ssh-keys',
  load: async () => ({ keys: await api.get('/api/ssh-keys') }),
  apply: (d) => { keys.value = d.keys },
  onError: () => { keys.value = [] },
})

// ---- add ----
const modalOpen = ref(false)
const form = ref({ name: '', private_key: '', password: '' })
const creating = ref(false)
const err = ref('')
const keyFile = ref(null)

// Name of the uploaded key file, if any. When set we DON'T render the key text — a
// private key shouldn't sit visible in the form; we just confirm it's loaded.
const keyFileName = ref('')

// Read a chosen key file into the (hidden) key field. The file never leaves the
// browser until you submit; it's sealed under your password server-side.
function onKeyFile(e) {
  const f = e.target.files?.[0]
  if (!f) return
  const r = new FileReader()
  r.onload = () => { form.value.private_key = String(r.result || '').trim(); keyFileName.value = f.name }
  r.readAsText(f)
  e.target.value = '' // allow re-picking the same file
}
function clearKey() { form.value.private_key = ''; keyFileName.value = '' }

function openNew() {
  form.value = { name: '', private_key: '', password: '' }
  keyFileName.value = ''
  err.value = ''
  modalOpen.value = true
}

async function create() {
  err.value = ''
  if (!form.value.name.trim()) { err.value = 'Give the key a name.'; return }
  if (!form.value.private_key.trim()) { err.value = 'Paste your private key.'; return }
  if (!form.value.password) { err.value = 'Enter your account password to encrypt the key.'; return }
  creating.value = true
  try {
    await api.post('/api/ssh-keys', {
      name: form.value.name.trim(),
      private_key: form.value.private_key,
      password: form.value.password,
    })
    modalOpen.value = false
    await load()
  } catch (e) {
    // Prefer the server's specific reason (wrong password / bad key / dup name).
    err.value = (e.body && e.body.trim())
      || (e.status === 409 ? 'A key with that name already exists.'
        : e.status === 400 ? 'Invalid key, inputs, or wrong account password.'
        : `Failed (${e.status}).`)
  } finally {
    creating.value = false
  }
}

async function remove(k) {
  if (!(await confirm({ title: `Delete "${k.name}"?`, message: 'This key is removed from your library. Any console session that would use it must pick another. This cannot be undone.', danger: true, confirmText: 'Delete' }))) return
  try { await api.del(`/api/ssh-keys/${k.id}`); await load() }
  catch (e) { err.value = `Failed to delete (${e.status}).` }
}

const fmt = (s) => (s ? new Date(s).toLocaleString([], { month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }) : '—')

onMounted(load)
</script>

<template>
  <AppShell title="SSH keys">
    <div class="space-y-4">
      <div class="flex items-start gap-3">
        <p class="max-w-3xl text-xs text-faint">Your personal library of SSH keys. Pick one when you open a host console — it's unsealed with your <b class="text-fg">account password</b> at connect time. Keys are encrypted at rest and never shown again after you add them.</p>
        <button @click="openNew" class="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">
          <VIcon name="plus" :size="16" />Add key
        </button>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint">
              <th class="px-4 py-3 font-medium">Name</th>
              <th class="px-4 py-3 font-medium">Fingerprint</th>
              <th class="px-4 py-3 font-medium">Added</th>
              <th class="px-4 py-3"></th>
            </tr></thead>
            <tbody>
              <tr v-if="!loaded"><td colspan="4"><PageLoader min-height="40vh" /></td></tr>
              <tr v-else-if="!keys.length"><td colspan="4" class="px-4 py-10 text-center text-muted">No SSH keys yet. Add one to use key-based console auth.</td></tr>
              <tr v-for="k in keys" :key="k.id" class="border-b border-line/60 last:border-0 hover:bg-surface2/50">
                <td class="px-4 py-3">
                  <span class="inline-flex items-center gap-2 text-fg"><VIcon name="ssh" :size="16" class="text-faint" />{{ k.name }}</span>
                </td>
                <td class="px-4 py-3 font-mono text-xs text-muted">{{ k.key_fingerprint }}</td>
                <td class="px-4 py-3 tabular-nums text-muted">{{ fmt(k.created_at) }}</td>
                <td class="px-4 py-3 text-right">
                  <button @click="remove(k)" class="rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-rose-400" v-tip="'Delete'">
                    <VIcon name="trash" :size="16" />
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>

    <!-- add modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="modalOpen = false">
      <div class="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div class="flex items-center gap-3 border-b border-line px-5 py-4">
          <h3 class="text-base font-semibold text-fg">Add SSH key</h3>
          <button @click="modalOpen = false" class="ml-auto rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg" aria-label="Close"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <form @submit.prevent="create" autocomplete="off" class="space-y-4 p-5">
          <!-- honeypot: absorbs Chrome's "username + password" autofill so it doesn't
               land in the real Name field (Chrome ignores autocomplete=off otherwise) -->
          <input type="text" name="username" autocomplete="username" class="hidden" tabindex="-1" aria-hidden="true" />
          <input type="password" autocomplete="current-password" class="hidden" tabindex="-1" aria-hidden="true" />
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Name</span>
            <input v-model="form.name" name="key-label" placeholder="e.g. laptop-ed25519, prod-deploy"
              autocomplete="off" autocapitalize="off" spellcheck="false" data-1p-ignore data-lpignore="true"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </label>
          <div class="block">
            <div class="mb-1.5 flex items-center gap-2">
              <span class="text-[11px] font-semibold uppercase tracking-wide text-faint">Private key</span>
              <button v-if="!keyFileName" type="button" @click="keyFile?.click()" class="ml-auto inline-flex items-center gap-1 rounded-md border border-line2 bg-surface2 px-2 py-1 text-[11px] text-muted hover:text-fg">
                <VIcon name="copy" :size="12" /> Upload file
              </button>
              <input ref="keyFile" type="file" class="hidden" @change="onKeyFile" />
            </div>
            <!-- uploaded from a file: confirm it's loaded, but never display the key -->
            <div v-if="keyFileName" class="flex items-center gap-2 rounded-lg border border-ok/30 bg-ok/10 px-3 py-2.5 text-sm">
              <VIcon name="check-circle" :size="16" class="shrink-0 text-ok" />
              <span class="min-w-0 flex-1 truncate font-mono text-fg">{{ keyFileName }}</span>
              <span class="shrink-0 text-xs text-faint">loaded · hidden</span>
              <button type="button" @click="clearKey" class="shrink-0 text-xs text-muted hover:text-down">Clear</button>
            </div>
            <!-- otherwise: paste -->
            <textarea v-else v-model="form.private_key" rows="6" spellcheck="false"
              placeholder="Paste your private key, or use Upload file&#10;-----BEGIN OPENSSH PRIVATE KEY-----"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-xs text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none"></textarea>
          </div>
          <label class="block">
            <span class="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-faint">Account password</span>
            <input v-model="form.password" type="password" autocomplete="new-password" data-1p-ignore data-lpignore="true"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
          </label>
          <p class="text-[11px] text-faint">Your key is encrypted with your account password — it can't be read without you.</p>
          <p v-if="err" class="text-xs text-rose-400">{{ err }}</p>
          <div class="flex justify-end gap-2.5 pt-1">
            <button type="button" @click="modalOpen = false" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
            <button type="submit" :disabled="creating" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ creating ? 'Adding…' : 'Add key' }}</button>
          </div>
        </form>
      </div>
    </div>
  </AppShell>
</template>
