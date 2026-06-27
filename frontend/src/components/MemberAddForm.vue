<script setup>
// Presentational add-member modal: collects email + temporary password and emits
// `submit`; after the parent creates the user it passes `created` back to show the
// credentials hand-off. The parent owns validation, the API call, `adding`/`error`.
import { ref, computed, watch } from 'vue'

const props = defineProps({
  adding: { type: Boolean, default: false },
  error: { type: String, default: '' },
  created: { type: Object, default: null }, // { email, password } after success
})
const emit = defineEmits(['submit', 'close'])

const nu = ref({ email: '', password: '' })
const showPw = ref(false)
const showCreatedPw = ref(false)

// Reset the form whenever the modal is (re)opened — i.e. when the parent clears
// `created` back to null for a fresh add.
watch(() => props.created, (v) => { if (!v) { nu.value = { email: '', password: '' }; showPw.value = false; showCreatedPw.value = false } })

function genPassword() {
  const chars = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
  const a = new Uint32Array(16); crypto.getRandomValues(a)
  nu.value.password = Array.from(a, (n) => chars[n % chars.length]).join('')
  showPw.value = true
}
function submit() { emit('submit', { email: nu.value.email, password: nu.value.password }) }

const credentialsText = computed(() => props.created ? `Vantage\nURL: ${location.origin}\nEmail: ${props.created.email}\nPassword: ${props.created.password}` : '')
function copyCreds(ev) {
  navigator.clipboard?.writeText(credentialsText.value)
  const b = ev.target, o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-start justify-center overflow-auto bg-black/65 p-4 backdrop-blur-sm sm:p-8" @click.self="emit('close')">
    <div class="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
      <div class="flex items-center gap-3 border-b border-line px-5 py-4">
        <h3 class="text-base font-semibold text-fg">{{ created ? 'Member created' : 'Add member' }}</h3>
        <button @click="emit('close')" class="ml-auto rounded-lg p-1.5 text-muted hover:bg-surface2 hover:text-fg"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
      </div>

      <!-- form -->
      <form v-if="!created" @submit.prevent="submit" class="space-y-4 p-5">
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
        <p v-if="error" class="text-xs text-rose-400">{{ error }}</p>
        <div class="flex justify-end gap-2.5 pt-1">
          <button type="button" @click="emit('close')" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
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
          <button @click="emit('close')" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Done</button>
        </div>
      </div>
    </div>
  </div>
</template>
