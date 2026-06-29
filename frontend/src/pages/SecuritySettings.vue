<script setup>
// Account security settings as a single-open accordion: each area (password,
// two-factor, SSH keys) is a collapsed row; clicking one expands it (and collapses
// the others) so the user focuses on one task at a time. Themed dialogs only — no
// native browser prompts.
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'
import { register as webauthnRegister, supported as webauthnSupported } from '../lib/webauthn'

const isAdmin = computed(() => !!useAuth().user?.is_admin)
const EXPOSURE_DOCS = 'https://github.com/The-Last-Devops/vantage/blob/main/docs/exposure.md'

// which section is expanded ('' = all collapsed)
const open = ref('')
function toggle(key) { open.value = open.value === key ? '' : key }

// ---- public exposure self-check (admin) ----
const exposure = ref(null) // { configured, public_url, exposed, status, error }
const exposureBusy = ref(false)
async function runExposureCheck() {
  exposureBusy.value = true
  try { exposure.value = await api.post('/api/admin/exposure-check') }
  catch (e) { exposure.value = { error: `Check failed (${e.status || 'error'})` } }
  finally { exposureBusy.value = false }
}

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
const enroll = ref(null)
const enrollCode = ref('')
const backupCodes = ref(null)
const tfaErr = ref('')
const tfaBusy = ref(false)
const disabling = ref(false) // inline "turn off" password prompt
const disablePw = ref('')

async function loadTfa() {
  try { tfa.value = await api.get('/api/me/2fa') } catch { /* defaults */ }
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
  if (!disablePw.value) return
  tfaErr.value = ''; tfaBusy.value = true
  try {
    await api.post('/api/me/2fa/disable', { password: disablePw.value })
    disabling.value = false; disablePw.value = ''; backupCodes.value = null
    await loadTfa()
  } catch (e) {
    tfaErr.value = e.status === 401 ? 'Wrong password.' : `Failed (${e.status || 'error'}).`
  } finally { tfaBusy.value = false }
}

// ---- passkeys (WebAuthn second factor) ----
const passkeys = ref([])
const pkErr = ref('')
const pkBusy = ref(false)
const addingPk = ref(false)
const pkName = ref('')

async function loadPasskeys() { try { passkeys.value = await api.get('/api/me/passkeys') } catch { passkeys.value = [] } }
onMounted(loadPasskeys)

async function addPasskey() {
  pkErr.value = ''
  const name = pkName.value.trim()
  if (!name) { pkErr.value = 'Give the passkey a name.'; return }
  if (!webauthnSupported()) { pkErr.value = 'This browser does not support passkeys.'; return }
  pkBusy.value = true
  try {
    const opts = await api.post('/api/me/passkeys/register/start')
    const credential = await webauthnRegister(opts)
    await api.post('/api/me/passkeys/register/finish', { name, credential })
    addingPk.value = false; pkName.value = ''
    await loadPasskeys()
  } catch (e) {
    pkErr.value = e?.name === 'NotAllowedError' ? 'Cancelled or timed out.'
      : e?.name === 'SecurityError' ? `This page's domain doesn't match the server's passkey settings — an admin must set WEBAUTHN_RP_ID to this domain and WEBAUTHN_ORIGIN to ${location.origin}. (${e.message || 'SecurityError'})`
      : e.status === 503 ? 'Passkeys are not configured on this server.'
      : `Couldn't register (${e?.message || e?.status || e?.name || 'error'}).`
  } finally { pkBusy.value = false }
}
async function deletePasskey(id) {
  pkErr.value = ''
  try { await api.del(`/api/me/passkeys/${id}`); await loadPasskeys() }
  catch (e) { pkErr.value = `Couldn't remove (${e.status || 'error'}).` }
}
</script>

<template>
  <AppShell title="Security">
    <div class="space-y-3">
      <!-- PASSWORD -->
      <section class="overflow-hidden rounded-xl border border-line bg-surface">
        <button @click="toggle('password')" class="flex w-full items-center gap-3 px-5 py-4 text-left hover:bg-surface2">
          <VIcon name="shield" :size="18" class="shrink-0 text-accent" />
          <div class="min-w-0 flex-1">
            <div class="text-h2 font-semibold text-fg">Password</div>
            <div class="text-xs text-muted">Change your account password.</div>
          </div>
          <VIcon name="chevron" :size="16" class="shrink-0 text-faint transition-transform" :class="open === 'password' ? 'rotate-90' : ''" />
        </button>
        <div v-show="open === 'password'" class="border-t border-line px-5 py-4">
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
            <p class="text-[11px] text-faint">At least 12 characters with upper, lower, and a digit. Your SSH keys keep working.</p>
            <p v-if="err" class="text-xs text-rose-400">{{ err }}</p>
            <p v-if="ok" class="text-xs text-ok">Password changed.</p>
            <button type="submit" :disabled="saving"
              class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ saving ? 'Saving…' : 'Change password' }}</button>
          </form>
        </div>
      </section>

      <!-- TWO-FACTOR -->
      <section class="overflow-hidden rounded-xl border border-line bg-surface">
        <button @click="toggle('twofa')" class="flex w-full items-center gap-3 px-5 py-4 text-left hover:bg-surface2">
          <VIcon name="shield" :size="18" class="shrink-0" :class="tfa.enabled ? 'text-ok' : 'text-muted'" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="text-h2 font-semibold text-fg">Two-factor authentication</span>
              <span v-if="tfaLoaded && tfa.enabled" class="rounded-full bg-ok/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-ok">On</span>
            </div>
            <div class="text-xs text-muted">A 6-digit authenticator code at sign-in (Google Authenticator, 1Password, Authy…).</div>
          </div>
          <VIcon name="chevron" :size="16" class="shrink-0 text-faint transition-transform" :class="open === 'twofa' ? 'rotate-90' : ''" />
        </button>
        <div v-show="open === 'twofa'" class="border-t border-line px-5 py-4">
          <p v-if="tfaErr" class="mb-3 text-xs text-rose-400">{{ tfaErr }}</p>

          <div v-if="backupCodes" class="mb-3 rounded-lg border border-ok/40 bg-ok/10 p-3">
            <p class="mb-2 text-xs font-semibold text-ok">Save these backup codes — each works once if you lose your authenticator.</p>
            <div class="grid grid-cols-2 gap-1.5 font-mono text-sm text-fg sm:grid-cols-5">
              <span v-for="c in backupCodes" :key="c" class="rounded bg-surface2 px-2 py-1 text-center">{{ c }}</span>
            </div>
          </div>

          <!-- enabled -->
          <template v-if="tfaLoaded && tfa.enabled">
            <p class="mb-3 text-xs text-muted">Two-factor auth is on. <span class="text-faint">{{ tfa.backup_codes_remaining }} backup code(s) left.</span></p>
            <div v-if="disabling" class="flex max-w-sm flex-wrap items-end gap-2">
              <label class="block flex-1">
                <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Account password</span>
                <input v-model="disablePw" type="password" autocomplete="current-password" autofocus @keyup.enter="disableTfa"
                  class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
              </label>
              <button :disabled="tfaBusy || !disablePw" @click="disableTfa"
                class="rounded-lg border border-rose-400/50 px-3 py-2.5 text-sm text-rose-400 hover:bg-rose-400/10 disabled:opacity-50">Turn off</button>
              <button :disabled="tfaBusy" @click="disabling = false; disablePw = ''" class="rounded-lg px-3 py-2.5 text-sm text-muted hover:text-fg">Cancel</button>
            </div>
            <button v-else :disabled="tfaBusy" @click="disabling = true"
              class="rounded-lg border border-line px-4 py-2 text-sm text-muted hover:border-rose-400/50 hover:text-rose-400 disabled:opacity-50">Turn off</button>
          </template>

          <!-- enrolling -->
          <template v-else-if="enroll">
            <p class="mb-3 text-xs text-muted">Scan this QR with your authenticator app, or enter the setup key manually:</p>
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
              <input v-model="enrollCode" inputmode="numeric" placeholder="123456" @keyup.enter="confirmEnroll"
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
        </div>
      </section>

      <!-- PASSKEYS -->
      <section class="overflow-hidden rounded-xl border border-line bg-surface">
        <button @click="toggle('passkeys')" class="flex w-full items-center gap-3 px-5 py-4 text-left hover:bg-surface2">
          <VIcon name="shield" :size="18" class="shrink-0" :class="passkeys.length ? 'text-ok' : 'text-muted'" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="text-h2 font-semibold text-fg">Passkeys</span>
              <span v-if="passkeys.length" class="rounded-full bg-ok/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-ok">{{ passkeys.length }}</span>
            </div>
            <div class="text-xs text-muted">Sign in with Touch ID / Windows Hello / a security key as a second factor.</div>
          </div>
          <VIcon name="chevron" :size="16" class="shrink-0 text-faint transition-transform" :class="open === 'passkeys' ? 'rotate-90' : ''" />
        </button>
        <div v-show="open === 'passkeys'" class="border-t border-line px-5 py-4">
          <p v-if="pkErr" class="mb-3 text-xs text-rose-400">{{ pkErr }}</p>
          <ul v-if="passkeys.length" class="mb-3 divide-y divide-line/60 overflow-hidden rounded-lg border border-line">
            <li v-for="p in passkeys" :key="p.id" class="flex items-center gap-3 px-3 py-2.5">
              <VIcon name="shield" :size="15" class="shrink-0 text-faint" />
              <div class="min-w-0 flex-1">
                <div class="truncate text-sm text-fg">{{ p.name }}</div>
                <div class="text-[11px] text-faint">Added {{ p.created_at?.slice(0, 10) }}</div>
              </div>
              <button @click="deletePasskey(p.id)" v-tip="'Remove passkey'"
                class="grid h-7 w-7 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-rose-400"><VIcon name="trash" :size="15" /></button>
            </li>
          </ul>
          <p v-else class="mb-3 text-xs text-faint">No passkeys yet.</p>

          <div v-if="addingPk" class="flex max-w-sm flex-wrap items-end gap-2">
            <label class="block flex-1">
              <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Passkey name</span>
              <input v-model="pkName" placeholder="MacBook Touch ID" autofocus @keyup.enter="addPasskey"
                class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
            </label>
            <button :disabled="pkBusy" @click="addPasskey"
              class="rounded-lg bg-accent px-4 py-2.5 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ pkBusy ? 'Waiting…' : 'Register' }}</button>
            <button :disabled="pkBusy" @click="addingPk = false; pkName = ''" class="rounded-lg px-3 py-2.5 text-sm text-muted hover:text-fg">Cancel</button>
          </div>
          <button v-else @click="addingPk = true; pkErr = ''"
            class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90">Add passkey</button>
        </div>
      </section>

      <!-- PUBLIC EXPOSURE (admin) -->
      <section v-if="isAdmin" class="overflow-hidden rounded-xl border border-line bg-surface">
        <button @click="toggle('exposure')" class="flex w-full items-center gap-3 px-5 py-4 text-left hover:bg-surface2">
          <VIcon name="globe" :size="18" class="shrink-0" :class="exposure && exposure.exposed ? 'text-rose-400' : exposure && exposure.exposed === false ? 'text-ok' : 'text-muted'" />
          <div class="min-w-0 flex-1">
            <div class="text-h2 font-semibold text-fg">Public exposure</div>
            <div class="text-xs text-muted">Is the hub reachable on the internet without an auth gate in front?</div>
          </div>
          <VIcon name="chevron" :size="16" class="shrink-0 text-faint transition-transform" :class="open === 'exposure' ? 'rotate-90' : ''" />
        </button>
        <div v-show="open === 'exposure'" class="border-t border-line px-5 py-4">
          <button :disabled="exposureBusy" @click="runExposureCheck"
            class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ exposureBusy ? 'Checking…' : 'Check now' }}</button>

          <div v-if="exposure" class="mt-3 max-w-prose text-sm">
            <p v-if="exposure.configured === false" class="text-warn">
              Set the <span class="font-mono text-cap">PUBLIC_URL</span> env to the hub's public address (e.g. <span class="font-mono text-cap">https://vantage.example.com</span>) to run this check.
            </p>
            <p v-else-if="exposure.error" class="text-muted">
              Couldn't reach <span class="font-mono">{{ exposure.public_url }}</span> — {{ exposure.error }}. It may already be gated or not publicly routable.
            </p>
            <template v-else-if="exposure.exposed">
              <p class="font-medium text-rose-400">⚠ Exposed — <span class="font-mono">{{ exposure.public_url }}</span> is reachable from the internet with no auth gate (HTTP {{ exposure.status }}).</p>
              <p class="mt-1 text-muted">Put the hub behind <b class="text-fg">nginx basic-auth</b> or <b class="text-fg">Cloudflare Zero Trust</b> — and allow <span class="font-mono text-cap">/pub/*</span> through so agents keep working.</p>
            </template>
            <p v-else class="font-medium text-ok">✓ Protected — the gate-less request was blocked (HTTP {{ exposure.status }}).</p>
            <a :href="EXPOSURE_DOCS" target="_blank" rel="noopener" class="mt-2 inline-block text-xs text-accent hover:underline">How to configure the gate (docs) →</a>
          </div>
          <p v-else class="mt-3 text-xs text-faint">Makes one outbound request to your <span class="font-mono text-cap">PUBLIC_URL</span> at a marker path outside <span class="font-mono text-cap">/pub</span>.</p>
        </div>
      </section>

      <!-- SSH KEYS -->
      <section class="overflow-hidden rounded-xl border border-line bg-surface">
        <button @click="toggle('ssh')" class="flex w-full items-center gap-3 px-5 py-4 text-left hover:bg-surface2">
          <VIcon name="ssh" :size="18" class="shrink-0 text-accent" />
          <div class="min-w-0 flex-1">
            <div class="text-h2 font-semibold text-fg">SSH keys</div>
            <div class="text-xs text-muted">Private keys for host consoles, sealed under your password.</div>
          </div>
          <VIcon name="chevron" :size="16" class="shrink-0 text-faint transition-transform" :class="open === 'ssh' ? 'rotate-90' : ''" />
        </button>
        <div v-show="open === 'ssh'" class="border-t border-line px-5 py-4">
          <p class="mb-3 max-w-prose text-xs text-muted">
            Manage the private keys you use to open host consoles. Keys are sealed under your password and never shown again after upload.
          </p>
          <RouterLink :to="{ name: 'ssh-keys' }"
            class="inline-flex items-center gap-2 rounded-lg border border-line px-4 py-2 text-sm text-fg hover:border-accent/50">
            <VIcon name="ssh" :size="15" /> Manage SSH keys
          </RouterLink>
        </div>
      </section>
    </div>
  </AppShell>
</template>
