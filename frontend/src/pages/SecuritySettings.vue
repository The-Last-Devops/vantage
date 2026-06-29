<script setup>
// Account security settings: change password (re-wraps the SSH-key master key so
// keys survive — see masterkey.rs), two-factor auth (TOTP), and a pointer to the
// SSH key library. A proper page rather than a cramped dropdown modal.
import { ref, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'

// ---- change password ----
const current = ref('')
const next = ref('')
const confirm = ref('')
const saving = ref(false)
const err = ref('')
const ok = ref(false)

function policyMsg(p) {
  if (p.length < 12) return 'At least 12 characters.'
  if (!/[a-z]/.test(p) || !/[A-Z]/.test(p) || !/[0-9]/.test(p)) return 'Mix upper, lower, and a digit.'
  return ''
}

async function changePassword() {
  err.value = ''; ok.value = false
  const pm = policyMsg(next.value)
  if (pm) { err.value = pm; return }
  if (next.value !== confirm.value) { err.value = 'New passwords do not match.'; return }
  saving.value = true
  try {
    await api.post('/api/me/password', { current_password: current.value, new_password: next.value })
    ok.value = true
    current.value = ''; next.value = ''; confirm.value = ''
  } catch (e) {
    err.value = e.status === 401 ? 'Current password is wrong.'
      : e.status === 400 ? 'New password does not meet the policy.'
      : `Failed (${e.status || 'error'}).`
  } finally {
    saving.value = false
  }
}

// ---- two-factor auth (TOTP) ----
const tfa = ref({ enabled: false, pending: false, backup_codes_remaining: 0 })
const tfaLoaded = ref(false)
const enroll = ref(null)   // { secret, otpauth_uri } while setting up
const enrollCode = ref('')
const backupCodes = ref(null) // shown once after enabling
const tfaErr = ref('')
const tfaBusy = ref(false)

async function loadTfa() {
  try { tfa.value = await api.get('/api/me/2fa') } catch { /* leave defaults */ }
  finally { tfaLoaded.value = true }
}
onMounted(loadTfa)

async function startEnroll() {
  tfaErr.value = ''; backupCodes.value = null; tfaBusy.value = true
  try { enroll.value = await api.post('/api/me/2fa/start'); enrollCode.value = '' }
  catch (e) { tfaErr.value = `Couldn't start setup (${e.status || 'error'}).` }
  finally { tfaBusy.value = false }
}
async function confirmEnroll() {
  tfaErr.value = ''; tfaBusy.value = true
  try {
    const r = await api.post('/api/me/2fa/enable', { code: enrollCode.value })
    backupCodes.value = r.backup_codes
    enroll.value = null
    await loadTfa()
  } catch (e) {
    tfaErr.value = e.status === 400 ? 'That code is wrong or expired — try the current one.' : `Failed (${e.status || 'error'}).`
  } finally { tfaBusy.value = false }
}
async function disableTfa() {
  const pw = window.prompt('Enter your account password to turn off two-factor auth:')
  if (!pw) return
  tfaErr.value = ''; tfaBusy.value = true
  try {
    await api.post('/api/me/2fa/disable', { password: pw })
    backupCodes.value = null
    await loadTfa()
  } catch (e) {
    tfaErr.value = e.status === 401 ? 'Wrong password.' : `Failed (${e.status || 'error'}).`
  } finally { tfaBusy.value = false }
}
</script>

<template>
  <AppShell title="Security">
    <div class="space-y-4">
      <!-- Password -->
      <section class="rounded-xl border border-line bg-surface p-5">
        <div class="mb-1 flex items-center gap-2">
          <VIcon name="shield" :size="16" class="text-accent" />
          <h2 class="text-h2 font-semibold text-fg">Password</h2>
        </div>
        <p class="mb-4 text-xs text-muted">
          Changing your password keeps your SSH keys working — they're re-secured under your new password automatically.
        </p>
        <form @submit.prevent="changePassword" autocomplete="off" class="max-w-sm space-y-3">
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Current password</span>
            <input v-model="current" type="password" autocomplete="current-password" required
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
          </label>
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">New password</span>
            <input v-model="next" type="password" autocomplete="new-password" required
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
          </label>
          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Confirm new password</span>
            <input v-model="confirm" type="password" autocomplete="new-password" required
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
          </label>
          <p class="text-[11px] text-faint">At least 12 characters with upper, lower, and a digit.</p>
          <p v-if="err" class="text-xs text-rose-400">{{ err }}</p>
          <p v-if="ok" class="text-xs text-ok">Password changed.</p>
          <button type="submit" :disabled="saving"
            class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ saving ? 'Saving…' : 'Change password' }}</button>
        </form>
      </section>

      <!-- Two-factor authentication (TOTP) -->
      <section class="rounded-xl border border-line bg-surface p-5">
        <div class="mb-1 flex items-center gap-2">
          <VIcon name="shield" :size="16" :class="tfa.enabled ? 'text-ok' : 'text-muted'" />
          <h2 class="text-h2 font-semibold text-fg">Two-factor authentication</h2>
          <span v-if="tfaLoaded && tfa.enabled" class="rounded-full bg-ok/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-ok">On</span>
        </div>
        <p class="mb-4 max-w-prose text-xs text-muted">
          Require a 6-digit code from an authenticator app (Google Authenticator, 1Password, Authy…) at sign-in, in addition to your password.
        </p>

        <p v-if="tfaErr" class="mb-3 text-xs text-rose-400">{{ tfaErr }}</p>

        <!-- backup codes (shown once right after enabling) -->
        <div v-if="backupCodes" class="mb-3 rounded-lg border border-ok/40 bg-ok/10 p-3">
          <p class="mb-2 text-xs font-semibold text-ok">Save these backup codes — each works once if you lose your authenticator.</p>
          <div class="grid grid-cols-2 gap-1.5 font-mono text-sm text-fg sm:grid-cols-5">
            <span v-for="c in backupCodes" :key="c" class="rounded bg-surface2 px-2 py-1 text-center">{{ c }}</span>
          </div>
        </div>

        <!-- enabled -->
        <template v-if="tfaLoaded && tfa.enabled">
          <p class="mb-3 text-xs text-muted">Two-factor auth is on. <span class="text-faint">{{ tfa.backup_codes_remaining }} backup code(s) left.</span></p>
          <button :disabled="tfaBusy" @click="disableTfa"
            class="rounded-lg border border-line px-4 py-2 text-sm text-muted hover:border-rose-400/50 hover:text-rose-400 disabled:opacity-50">Turn off</button>
        </template>

        <!-- enrolling: show secret + code field -->
        <template v-else-if="enroll">
          <p class="mb-3 text-xs text-muted">Scan this QR with your authenticator app (Google Authenticator, 1Password, Authy…), or enter the setup key manually:</p>
          <div class="mb-3 flex flex-wrap items-start gap-4">
            <div v-if="enroll.qr_svg" class="shrink-0 rounded-lg bg-white p-2" v-html="enroll.qr_svg"></div>
            <div class="min-w-0">
              <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Setup key</span>
              <div class="mb-2 break-all rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-sm text-fg">{{ enroll.secret }}</div>
              <div class="break-all text-[11px] text-faint">{{ enroll.otpauth_uri }}</div>
            </div>
          </div>
          <label class="mb-3 block max-w-[220px]">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Code from the app</span>
            <input v-model="enrollCode" inputmode="numeric" placeholder="123456"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-center font-mono text-lg tracking-[0.3em] text-fg focus:border-accent/60 focus:outline-none" />
          </label>
          <div class="flex gap-2">
            <button :disabled="tfaBusy" @click="confirmEnroll"
              class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ tfaBusy ? 'Verifying…' : 'Verify & enable' }}</button>
            <button :disabled="tfaBusy" @click="enroll = null" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
          </div>
        </template>

        <!-- off -->
        <template v-else>
          <button :disabled="tfaBusy || !tfaLoaded" @click="startEnroll"
            class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">Set up authenticator</button>
        </template>
      </section>

      <!-- SSH keys -->
      <section class="rounded-xl border border-line bg-surface p-5">
        <div class="mb-1 flex items-center gap-2">
          <VIcon name="ssh" :size="16" class="text-accent" />
          <h2 class="text-h2 font-semibold text-fg">SSH keys</h2>
        </div>
        <p class="mb-4 max-w-prose text-xs text-muted">
          Manage the private keys you use to open host consoles. Keys are sealed under your password and never shown again after upload.
        </p>
        <RouterLink :to="{ name: 'ssh-keys' }"
          class="inline-flex items-center gap-2 rounded-lg border border-line px-4 py-2 text-sm text-fg hover:border-accent/50">
          <VIcon name="ssh" :size="15" /> Manage SSH keys
        </RouterLink>
      </section>
    </div>
  </AppShell>
</template>
