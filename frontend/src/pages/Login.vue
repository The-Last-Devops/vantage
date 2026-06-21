<script setup>
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const route = useRoute()
const router = useRouter()

const email = ref('')
const password = ref('')
const confirm = ref('')
const error = ref('')
const busy = ref(false)

const setup = () => auth.needsSetup

async function submit() {
  error.value = ''
  if (setup()) {
    if (password.value.length < 6) { error.value = 'Password must be at least 6 characters'; return }
    if (password.value !== confirm.value) { error.value = 'Passwords do not match'; return }
  }
  busy.value = true
  try {
    if (setup()) await auth.createAdmin(email.value, password.value)
    else await auth.login(email.value, password.value)
    router.push(route.query.next || { name: 'systems' })
  } catch (e) {
    error.value = setup()
      ? 'Could not create admin (maybe one already exists)'
      : e.status === 401 ? 'Invalid email or password' : 'Login failed'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="flex min-h-screen items-center justify-center px-6">
    <div class="w-full max-w-sm">
      <div class="mb-8 flex items-center justify-center gap-2.5">
        <span class="inline-block h-7 w-7 rounded-md bg-accent shadow-[0_0_20px_-4px_rgb(var(--accent))]"></span>
        <span class="text-xl font-semibold tracking-tight text-fg">last-monitor</span>
      </div>

      <form class="space-y-4 rounded-xl border border-line bg-surface p-7 shadow-2xl" @submit.prevent="submit">
        <div>
          <h1 class="text-base font-semibold text-fg">{{ setup() ? 'Create admin account' : 'Sign in' }}</h1>
          <p class="mt-1 text-sm text-muted">
            {{ setup() ? 'First run — set up the administrator account.' : 'Monitor your fleet & services.' }}
          </p>
        </div>

        <label class="block text-sm">
          <span class="text-muted">Email</span>
          <input v-model="email" type="email" required autocomplete="username"
            class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
        </label>

        <label class="block text-sm">
          <span class="text-muted">Password</span>
          <input v-model="password" type="password" required :autocomplete="setup() ? 'new-password' : 'current-password'"
            class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
        </label>

        <label v-if="setup()" class="block text-sm">
          <span class="text-muted">Confirm password</span>
          <input v-model="confirm" type="password" required autocomplete="new-password"
            class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
        </label>

        <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

        <button type="submit" :disabled="busy"
          class="w-full rounded-lg bg-accent px-4 py-2.5 font-semibold text-accentfg transition hover:opacity-90 disabled:opacity-50">
          {{ busy ? 'Working…' : setup() ? 'Create account' : 'Sign in' }}
        </button>

        <p v-if="!setup()" class="text-center text-xs text-faint">No public registration — accounts are provisioned by an admin.</p>
      </form>
    </div>
  </div>
</template>
