<script setup>
import { ref, computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuth } from '../stores/auth'
import { passwordProblem } from '../lib/password'
import { authenticate as webauthnAuthenticate } from '../lib/webauthn'

const auth = useAuth()
const route = useRoute()
const router = useRouter()

const email = ref('')
const password = ref('')
const confirm = ref('')
const error = ref('')
const busy = ref(false)
const twofa = ref(false)   // second step: the account has 2FA
const totpCode = ref('')
const hasTotp = ref(false)      // account has an authenticator
const pkChallenge = ref(null)   // passkey assertion challenge, if the account has passkeys

const setup = () => auth.needsSetup

// Live policy hint while creating the admin (empty once the password is acceptable).
const pwHint = computed(() => (setup() && password.value ? passwordProblem(password.value) : ''))

// Detailed failure reason — this is an internal console, so be explicit (API down,
// timeout, 5xx, and the server's own message body).
function describeErr(e) {
  if (e && e.status != null) {
    const body = (e.body || '').toString().trim().slice(0, 240)
    const base = {
      400: 'bad request (400)', 403: 'forbidden (403)', 429: 'rate limited (429)',
      500: 'server error (500)', 502: 'bad gateway (502)',
      503: 'service unavailable (503) — server starting or a dependency (DB) is down',
      504: 'gateway timeout (504)',
    }[e.status] || `HTTP ${e.status}`
    return body ? `${base} — ${body}` : base
  }
  const m = (e && e.message) || ''
  if (/abort|timeout/i.test(m)) return 'request timed out'
  return `cannot reach the server (API down?) — ${m || 'network error'}`
}

async function submit() {
  error.value = ''
  if (setup()) {
    const problem = passwordProblem(password.value)
    if (problem) { error.value = problem; return }
    if (password.value !== confirm.value) { error.value = 'Passwords do not match'; return }
  }
  busy.value = true
  try {
    if (setup()) {
      await auth.createAdmin(email.value, password.value)
    } else {
      const res = await auth.login(email.value, password.value, twofa.value ? { totpCode: totpCode.value } : {})
      if (res.twofaRequired) { // show the 2FA step (TOTP code and/or passkey)
        twofa.value = true; hasTotp.value = res.totp; pkChallenge.value = res.passkey
        busy.value = false; return
      }
    }
    router.push(route.query.next || { name: 'systems' })
  } catch (e) {
    error.value = setup()
      ? (e.status === 403 ? 'An admin already exists.' : `Could not create admin — ${describeErr(e)}`)
      : twofa.value
        ? (e.status === 401 ? 'Invalid or expired code' : `Verification failed — ${describeErr(e)}`)
        : e.status === 401 ? 'Invalid email or password'
        : `Sign-in failed — ${describeErr(e)}`
  } finally {
    busy.value = false
  }
}

async function usePasskey() {
  if (!pkChallenge.value) return
  error.value = ''; busy.value = true
  try {
    const cred = await webauthnAuthenticate(pkChallenge.value)
    const res = await auth.login(email.value, password.value, { passkeyCredential: cred })
    if (res.twofaRequired) { error.value = 'Passkey was not accepted'; return }
    router.push(route.query.next || { name: 'systems' })
  } catch (e) {
    error.value = e?.name === 'NotAllowedError' ? 'Passkey cancelled or timed out' : `Passkey sign-in failed — ${describeErr(e)}`
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="flex min-h-screen items-center justify-center px-6">
    <div class="w-full max-w-sm">
      <div class="mb-8 flex items-center justify-center gap-2.5">
        <span class="vantage-logo grid h-9 w-9 place-items-center rounded-lg">
          <svg viewBox="0 0 64 64" class="h-[22px] w-[22px]" fill="none" stroke="#08231F" stroke-width="5.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M7 43 L21 21 L35 43" />
            <rect x="42" y="37.5" width="14" height="6" rx="1.8" fill="#08231F" stroke="none" />
          </svg>
        </span>
        <span class="text-xl font-semibold tracking-tight text-fg">Vantage</span>
      </div>

      <form class="space-y-4 rounded-xl border border-line bg-surface p-7 shadow-2xl" @submit.prevent="submit">
        <div>
          <h1 class="text-base font-semibold text-fg">{{ setup() ? 'Create admin account' : twofa ? 'Two-factor authentication' : 'Sign in' }}</h1>
          <p class="mt-1 text-sm text-muted">
            {{ setup() ? 'First run — set up the administrator account.' : twofa ? 'Confirm it\'s you with a second factor.' : 'Monitor your fleet & services.' }}
          </p>
        </div>

        <!-- step 2: 2FA (passkey and/or TOTP) -->
        <template v-if="twofa">
          <button v-if="pkChallenge" type="button" @click="usePasskey" :disabled="busy"
            class="flex w-full items-center justify-center gap-2 rounded-lg border border-accent/50 bg-accent/10 px-4 py-2.5 font-semibold text-accent transition hover:bg-accent/20 disabled:opacity-50">
            <VIcon name="shield" :size="16" /> Use a passkey
          </button>
          <div v-if="pkChallenge && hasTotp" class="flex items-center gap-3 text-xs text-faint">
            <span class="h-px flex-1 bg-line"></span>or enter a code<span class="h-px flex-1 bg-line"></span>
          </div>
          <label v-if="hasTotp" class="block text-sm">
            <span class="text-muted">Authentication code</span>
            <input v-model="totpCode" inputmode="numeric" autocomplete="one-time-code" autofocus placeholder="123456"
              class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-center font-mono text-lg tracking-[0.3em] text-fg outline-none transition focus:border-accent" />
            <span class="mt-1 block text-xs text-faint">Or enter one of your backup codes.</span>
          </label>
        </template>

        <!-- step 1: credentials -->
        <template v-else>
          <label class="block text-sm">
            <span class="text-muted">Email</span>
            <input v-model="email" type="email" required autocomplete="username"
              class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
          </label>

          <label class="block text-sm">
            <span class="text-muted">Password</span>
            <input v-model="password" type="password" required :autocomplete="setup() ? 'new-password' : 'current-password'"
              class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
            <span v-if="pwHint" class="mt-1 block text-xs text-warn">{{ pwHint }}</span>
            <span v-else-if="setup()" class="mt-1 block text-xs text-faint">12+ chars, mix of cases, digits &amp; symbols.</span>
          </label>

          <label v-if="setup()" class="block text-sm">
            <span class="text-muted">Confirm password</span>
            <input v-model="confirm" type="password" required autocomplete="new-password"
              class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-fg outline-none transition focus:border-accent" />
          </label>
        </template>

        <p v-if="error" class="text-sm text-red-500">{{ error }}</p>

        <button v-if="!twofa || hasTotp" type="submit" :disabled="busy"
          class="w-full rounded-lg bg-accent px-4 py-2.5 font-semibold text-accentfg transition hover:opacity-90 disabled:opacity-50">
          {{ busy ? 'Working…' : setup() ? 'Create account' : twofa ? 'Verify' : 'Sign in' }}
        </button>
        <button v-if="twofa" type="button" @click="twofa = false; totpCode = ''; pkChallenge = null; hasTotp = false; error = ''"
          class="w-full text-center text-xs text-muted hover:text-fg">← Back</button>

        <p v-if="!setup()" class="text-center text-xs text-faint">No public registration — accounts are provisioned by an admin.</p>
      </form>
    </div>
  </div>
</template>
