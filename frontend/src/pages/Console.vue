<script setup>
// Interactive SSH console for a host. Flow:
//   1. GET .../shell — gate on can_exec / tunnel_online (no stored credential).
//   2. Step-up: pick SSH user + auth method (host password, or a key from your library
//      unsealed with your account password) → POST .../console/ticket → ticket.
//   3. Open WS .../console?ticket=<id>, pipe xterm <-> binary stdout/stdin, JSON resize.
// On close → "Session ended" + Reconnect (re-does the ticket step).
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useRoute, useRouter, onBeforeRouteLeave } from 'vue-router'
import { confirm } from '../lib/confirm'
import { useUi } from '../stores/ui'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'

const route = useRoute()
const router = useRouter()
const ui = useUi()
// Re-skin a live terminal when the light/dark theme toggles — xterm reads its
// colours once at init, so reapply the CSS-var-derived theme on every change.
watch(
  () => ui.light,
  () => {
    if (term) term.options.theme = termTheme()
  },
)
const id = computed(() => route.params.id)
// preserve the workspace selection on the breadcrumb links
const nsq = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
const resolvedName = ref('') // host name looked up from /api/systems (precheck)
const hostName = computed(() => route.query.name || resolvedName.value || `Host ${String(id.value).slice(0, 8)}…`)

// shell precheck state
const shell = ref(null)
const loaded = ref(false)
const loadErr = ref('')

// ui phase: 'precheck' | 'blocked' | 'auth' | 'connecting' | 'live' | 'ended'
const phase = ref('precheck')
const blockedReason = ref('') // human text when blocked
const endReason = ref('')

// ---- step-up auth form ----
const sshUser = ref('')
const authMethod = ref('password') // 'password' | 'key'
const sshPassword = ref('')         // host password (method=password)
const accountPassword = ref('')     // account password to unseal a key (method=key)
const keyId = ref(null)             // chosen key from the library (method=key)
const keys = ref([])                // caller's key library, fetched when the modal opens
const authErr = ref('')
const submitting = ref(false)

const keyOptions = computed(() =>
  keys.value.map((k) => ({ value: k.id, label: `${k.name} (${k.key_fingerprint})` })),
)
const hasKeys = computed(() => keys.value.length > 0)
const methodOptions = computed(() => [
  { value: 'password', label: 'User + password' },
  { value: 'key', label: hasKeys.value ? 'User + key' : 'User + key (no keys yet)' },
])

async function loadKeys() {
  try {
    keys.value = await api.get('/api/ssh-keys')
  } catch {
    keys.value = []
  }
  // default the key picker to the first key if any; if no keys, force password method
  if (keys.value.length) { if (keyId.value == null) keyId.value = keys.value[0].id }
  else if (authMethod.value === 'key') authMethod.value = 'password'
}

// xterm
const termEl = ref(null)
let term = null
let fit = null
let ws = null
let ro = null

async function precheck() {
  try {
    shell.value = await api.get(`/api/systems/${id.value}/shell`)
    loadErr.value = ''
    // resolve the host's display name (the route only carries its id)
    if (!route.query.name && !resolvedName.value) {
      try { const sys = await api.get('/api/systems'); resolvedName.value = sys.find((s) => s.id === id.value)?.name || '' } catch {}
    }
    const s = shell.value
    // shell is always available now (the per-host enable/disable flag was removed);
    // access is gated only by RBAC (can_exec) + a live agent tunnel.
    if (!s.can_exec) { phase.value = 'blocked'; blockedReason.value = "You don't have shell access on this host." }
    else if (!s.tunnel_online) { phase.value = 'blocked'; blockedReason.value = 'Agent offline — the host is not reachable right now.' }
    else { phase.value = 'auth'; loadKeys() }
  } catch (e) {
    loadErr.value = e.status === 403 ? "You don't have shell access on this host." : 'Failed to load shell settings.'
    phase.value = 'blocked'
    blockedReason.value = loadErr.value
  } finally {
    loaded.value = true
  }
}

// xterm theme — pull literal colours from the live CSS-var tokens (xterm needs
// concrete strings, same approach as the uPlot charts).
function cssColor(name, fallback) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v ? `rgb(${v})` : fallback
}
function termTheme() {
  return {
    background: cssColor('--bg', '#0B0E14'),
    foreground: cssColor('--fg', '#E2E8F0'),
    cursor: cssColor('--accent', '#34E1C4'),
    cursorAccent: cssColor('--bg', '#0B0E14'),
    selectionBackground: cssColor('--accent', '#34E1C4') + '40',
  }
}

function sendResize() {
  if (ws && ws.readyState === WebSocket.OPEN && term) {
    ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }))
  }
}

async function connect() {
  authErr.value = ''
  if (!sshUser.value.trim()) { authErr.value = 'Enter the SSH user.'; return }
  let body
  if (authMethod.value === 'password') {
    if (!sshPassword.value) { authErr.value = 'Enter the SSH host password.'; return }
    body = { ssh_user: sshUser.value.trim(), auth: 'password', ssh_password: sshPassword.value }
  } else {
    if (!hasKeys.value) { authErr.value = 'Add a key in SSH keys first.'; return }
    if (keyId.value == null) { authErr.value = 'Choose a key.'; return }
    if (!accountPassword.value) { authErr.value = 'Enter your account password to unseal the key.'; return }
    body = { ssh_user: sshUser.value.trim(), auth: 'key', key_id: keyId.value, account_password: accountPassword.value }
  }
  submitting.value = true
  let ticket
  try {
    const res = await api.post(`/api/systems/${id.value}/console/ticket`, body)
    ticket = res.ticket
  } catch (e) {
    authErr.value = statusMsg(e.status)
    submitting.value = false
    return
  }
  // clear the typed secrets once consumed
  sshPassword.value = ''
  accountPassword.value = ''
  submitting.value = false
  phase.value = 'connecting'
  await nextTick()
  openSocket(ticket)
}

function statusMsg(status) {
  if (status === 403) return "You don't have shell access on this host."
  if (status === 400) return 'Could not start a session — check the SSH user, password/key, or the agent may be offline.'
  return `Failed to get a session (${status}).`
}

function openSocket(ticket) {
  // (re)create the terminal fresh each connection
  if (term) { term.dispose(); term = null }
  ensureTerm()

  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  // pass the fitted size so the PTY opens full-size (not a cramped 80x24)
  const size = term?.cols ? `&cols=${term.cols}&rows=${term.rows}` : ''
  const url = `${scheme}//${location.host}/api/systems/${id.value}/console?ticket=${encodeURIComponent(ticket)}${size}`
  ws = new WebSocket(url)
  ws.binaryType = 'arraybuffer'

  ws.onopen = () => {
    phase.value = 'live'
    nextTick(() => { try { fit?.fit() } catch {} ; sendResize(); term?.focus() })
  }
  ws.onmessage = (ev) => {
    if (ev.data instanceof ArrayBuffer) term?.write(new Uint8Array(ev.data))
    else if (typeof ev.data === 'string') term?.write(ev.data)
  }
  ws.onclose = (ev) => {
    if (phase.value !== 'ended') {
      endReason.value = ev.reason || (phase.value === 'connecting' ? 'Could not connect to the session.' : 'Session ended.')
      phase.value = 'ended'
    }
    term?.writeln('\r\n\x1b[90m— ' + (endReason.value || 'Session ended.') + ' —\x1b[0m')
  }
  ws.onerror = () => {
    if (phase.value === 'connecting') { endReason.value = 'Could not connect to the session.'; phase.value = 'ended' }
  }
}

function ensureTerm() {
  term = new Terminal({
    cursorBlink: true,
    fontSize: 13,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
    theme: termTheme(),
    scrollback: 5000,
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  term.open(termEl.value)
  try { fit.fit() } catch {}
  // browser → hub: terminal input as Binary bytes
  term.onData((data) => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(new TextEncoder().encode(data))
  })
  // refit + tell the hub on any size change
  ro = new ResizeObserver(() => { try { fit?.fit() } catch {} ; sendResize() })
  ro.observe(termEl.value)
}

function disconnect() {
  endReason.value = 'Disconnected.'
  phase.value = 'ended'
  if (ws) { try { ws.close() } catch {} ; ws = null }
}

function reconnect() {
  endReason.value = ''
  // re-do the precheck (tunnel may have dropped) then the password step
  loaded.value = false
  phase.value = 'precheck'
  precheck()
}

function cleanup() {
  if (ro) { try { ro.disconnect() } catch {} ; ro = null }
  if (ws) { try { ws.close() } catch {} ; ws = null }
  if (term) { try { term.dispose() } catch {} ; term = null }
}

function onWinResize() { try { fit?.fit() } catch {} ; sendResize() }

// Warn before leaving while a session is live — closing/reloading the tab (native
// prompt) or navigating away in-app (themed confirm) would drop the SSH connection.
function beforeUnload(e) { if (phase.value === 'live') { e.preventDefault(); e.returnValue = '' } }
onBeforeRouteLeave(async () => {
  if (phase.value !== 'live') return true
  return await confirm({
    title: 'Leave the console?',
    message: 'Your SSH session is still connected and will be closed if you leave.',
    confirmText: 'Leave',
    danger: true,
  })
})

onMounted(() => { precheck(); window.addEventListener('resize', onWinResize); window.addEventListener('beforeunload', beforeUnload) })
onBeforeUnmount(() => { window.removeEventListener('resize', onWinResize); window.removeEventListener('beforeunload', beforeUnload); cleanup() })

function back() { router.push(`/system/${id.value}`) }
</script>

<template>
  <!-- The console lives INSIDE the app content frame (nav/header stay visible) rather
       than full-bleed. It's opened in a new browser tab from the host page. The panel
       owns a bounded height so the terminal scrolls internally and htop/top fill it. -->
  <AppShell
    :title="hostName"
    :breadcrumb="[
      { label: 'Infrastructure', to: { name: 'systems', query: nsq } },
      { label: hostName, to: { name: 'system', params: { id }, query: nsq } },
      { label: 'Console' },
    ]">
    <template #header>
      <span class="flex items-center gap-1.5 font-mono text-xs">
        <span class="h-1.5 w-1.5 rounded-full" :class="phase === 'live' ? 'bg-accent' : 'bg-faint'"></span>
        <span :class="phase === 'live' ? 'text-accent' : 'text-faint'">{{ phase === 'live' ? 'Connected' : phase === 'ended' ? 'Ended' : phase === 'connecting' ? 'Connecting…' : 'SSH' }}</span>
      </span>
    </template>
    <template #actions>
      <button v-if="phase === 'live'" @click="disconnect"
        class="rounded-lg border border-line bg-surface2 px-3 py-1.5 text-xs text-muted hover:border-down/50 hover:text-down">Disconnect</button>
      <button v-else-if="phase === 'ended'" @click="reconnect"
        class="rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accentfg hover:opacity-90">Reconnect</button>
    </template>

    <div class="flex h-[calc(100dvh-9.5rem)] min-h-[420px] flex-col overflow-hidden rounded-xl border border-line bg-bg">
      <!-- precheck loader -->
      <div v-if="phase === 'precheck'" class="flex flex-1 items-center justify-center">
        <span class="text-sm text-muted">Loading…</span>
      </div>

      <!-- blocked states -->
      <div v-else-if="phase === 'blocked'" class="flex flex-1 items-center justify-center p-6">
        <div class="max-w-md rounded-xl border border-line bg-surface p-6 text-center">
          <p class="text-sm text-muted">{{ blockedReason }}</p>
          <button @click="back" class="mt-4 rounded-lg border border-line bg-surface2 px-4 py-2 text-sm text-fg hover:border-accent/50">Back to host</button>
        </div>
      </div>

      <!-- step-up auth modal: SSH user + (host password | key from library) -->
      <div v-else-if="phase === 'auth'" class="flex flex-1 items-center justify-center p-6">
        <form @submit.prevent="connect" autocomplete="off" class="w-full max-w-sm rounded-xl border border-line bg-surface p-6">
          <!-- honeypot: soaks up Chrome's username+password autofill so it doesn't land
               in the SSH user / password fields below -->
          <input type="text" name="username" autocomplete="username" class="hidden" tabindex="-1" aria-hidden="true" />
          <input type="password" autocomplete="current-password" class="hidden" tabindex="-1" aria-hidden="true" />
          <p class="mb-1 text-sm font-semibold text-fg">Connect to {{ hostName }}</p>
          <p class="mb-4 text-xs text-muted">Choose how to authenticate this SSH session. Nothing typed here is stored.</p>

          <label class="block">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">SSH user</span>
            <input v-model="sshUser" autofocus placeholder="e.g. ubuntu, root"
              autocomplete="off" autocapitalize="off" spellcheck="false" data-1p-ignore data-lpignore="true"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
          </label>

          <div class="mt-3">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Auth method</span>
            <UiSelect v-model="authMethod" block :options="methodOptions" />
          </div>

          <!-- password method -->
          <label v-if="authMethod === 'password'" class="mt-3 block">
            <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">SSH host password</span>
            <input v-model="sshPassword" type="password" autocomplete="new-password" data-1p-ignore data-lpignore="true"
              class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
          </label>

          <!-- key method -->
          <template v-else>
            <div v-if="!hasKeys" class="mt-3 rounded-lg border border-line bg-surface2 p-3 text-xs text-muted">
              You have no SSH keys yet.
              <router-link :to="{ name: 'ssh-keys' }" class="text-accent hover:underline">Add a key in SSH keys</router-link>.
            </div>
            <template v-else>
              <div class="mt-3">
                <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Key</span>
                <UiSelect v-model="keyId" block :options="keyOptions" placeholder="Choose a key…" />
              </div>
              <label class="mt-3 block">
                <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">Account password</span>
                <input v-model="accountPassword" type="password" autocomplete="new-password" data-1p-ignore data-lpignore="true"
                  class="w-full rounded-lg border border-line bg-surface2 px-3 py-2.5 text-sm text-fg focus:border-accent/60 focus:outline-none" />
                <span class="mt-1 block text-[11px] text-faint">Unseals your chosen key — it can't be read without you.</span>
              </label>
            </template>
          </template>

          <p v-if="authErr" class="mt-2 text-xs text-down">{{ authErr }}</p>
          <div class="mt-4 flex justify-end gap-2.5">
            <button type="button" @click="back" class="rounded-lg px-3 py-2 text-sm text-muted hover:text-fg">Cancel</button>
            <button type="submit" :disabled="submitting" class="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ submitting ? 'Connecting…' : 'Connect' }}</button>
          </div>
        </form>
      </div>

      <!-- terminal (kept mounted for connecting/live/ended so xterm keeps its buffer) -->
      <div v-show="['connecting', 'live', 'ended'].includes(phase)" class="min-h-0 flex-1 bg-bg p-2">
        <div ref="termEl" class="h-full w-full"></div>
      </div>
    </div>
  </AppShell>
</template>
