<script setup>
// Shell settings for a host: shell enable/port (owner-only), the online/tunnel
// status, and an "Open console" launcher. SSH credentials are no longer stored
// per-system — the caller picks their SSH user + password or one of their
// account-level keys at connect time (see Console.vue / SshKeys.vue). Parent
// passes the system id + display name; this owns its own fetch of GET .../shell
// so it can refresh after a mutation without touching the parent's metric polling.
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../lib/api'

const props = defineProps({
  id: { required: true },
  name: { type: String, default: '' },
})

const router = useRouter()

// shell state from the API:
//   { shell_enabled, ssh_port, tunnel_online, can_exec, has_keys }
const shell = ref(null)
const loaded = ref(false)
const loadErr = ref('')

async function load() {
  try {
    shell.value = await api.get(`/api/systems/${props.id}/shell`)
    loadErr.value = ''
  } catch (e) {
    if (e.status === 403) { shell.value = { can_exec: false } } // treated as "no access" below
    else loadErr.value = 'Failed to load shell settings.'
  } finally {
    loaded.value = true
  }
}
onMounted(load)

const canExec = computed(() => !!shell.value?.can_exec)
const canOpenConsole = computed(
  () => canExec.value && shell.value?.shell_enabled && shell.value?.tunnel_online,
)

// ---- owner-only: enable/disable shell + ssh port ----
const portInput = ref(22)
const savingShell = ref(false)
const shellMsg = ref('')
function syncPort() { portInput.value = shell.value?.ssh_port || 22 }

async function saveShell(enabled) {
  shellMsg.value = ''
  const port = Number(portInput.value)
  if (!Number.isInteger(port) || port < 1 || port > 65535) { shellMsg.value = 'Port must be 1–65535.'; return }
  savingShell.value = true
  try {
    await api.put(`/api/systems/${props.id}/shell`, { shell_enabled: enabled, ssh_port: port })
    await load(); syncPort()
  } catch (e) {
    shellMsg.value = e.status === 403 ? 'Only the namespace owner can change this.' : `Failed (${e.status}).`
  } finally {
    savingShell.value = false
  }
}

function openConsole() {
  router.push({ name: 'console', params: { id: props.id }, query: { name: props.name } })
}

// keep the port input in sync once data lands
watch(shell, syncPort)
</script>

<template>
  <div class="mb-4 rounded-xl border border-line bg-surface p-4">
    <div class="mb-3 flex items-center gap-2">
      <span class="text-[11px] font-semibold uppercase tracking-wider text-faint">Shell</span>
      <span v-if="loaded && shell?.shell_enabled" class="rounded-full bg-surface2 px-2 py-0.5 text-[10px] text-accent">enabled</span>
      <span v-else-if="loaded && canExec" class="rounded-full bg-surface2 px-2 py-0.5 text-[10px] text-faint">disabled</span>
      <button v-if="canOpenConsole" @click="openConsole"
        class="ml-auto inline-flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accentfg hover:opacity-90">
        <VIcon name="terminal" :size="14" />
        Open console
      </button>
    </div>

    <p v-if="!loaded" class="text-xs text-faint">Loading…</p>
    <p v-else-if="loadErr" class="text-xs text-rose-400">{{ loadErr }}</p>

    <!-- no exec permission -->
    <p v-else-if="!canExec" class="text-xs text-faint">You don't have shell access on this host.</p>

    <template v-else>
      <!-- status line -->
      <div class="mb-3 flex flex-wrap items-center gap-x-5 gap-y-1.5 text-xs">
        <span><span class="text-faint">SSH port</span> <span class="text-fg tabular-nums">{{ shell.ssh_port }}</span></span>
        <span class="flex items-center gap-1.5">
          <span class="h-1.5 w-1.5 rounded-full" :class="shell.tunnel_online ? 'bg-ok' : 'bg-faint'"></span>
          <span :class="shell.tunnel_online ? 'text-fg' : 'text-faint'">{{ shell.tunnel_online ? 'Shell channel ready' : 'Shell channel offline' }}</span>
        </span>
      </div>

      <!-- the host pushes metrics over one path; the interactive shell needs a SECOND,
           opt-in tunnel. Explain the gap so "offline" here isn't confused with the host. -->
      <p v-if="shell.shell_enabled && !shell.tunnel_online" class="mb-3 text-xs text-faint">
        This host reports metrics fine, but no shell tunnel is connected. Redeploy its agent with
        <span class="font-mono text-cap">ALLOW_SHELL=1</span> (and make sure the host runs sshd) to open the console.
      </p>

      <!-- owner-only enable/port form. We show it to anyone with exec and let a 403 surface
           a message, since the API doesn't return an explicit owner flag. -->
      <div class="mb-3 flex flex-wrap items-end gap-3 rounded-lg border border-line bg-surface2 p-3">
        <label class="block">
          <span class="mb-1 block text-[11px] font-semibold uppercase tracking-wide text-faint">SSH port</span>
          <input v-model.number="portInput" type="number" min="1" max="65535"
            class="w-28 rounded-lg border border-line bg-bg px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" />
        </label>
        <div class="flex gap-2">
          <button v-if="!shell.shell_enabled" :disabled="savingShell" @click="saveShell(true)"
            class="rounded-lg bg-accent px-3 py-2 text-sm font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ savingShell ? 'Saving…' : 'Enable shell' }}</button>
          <template v-else>
            <button :disabled="savingShell" @click="saveShell(true)"
              class="rounded-lg border border-line bg-bg px-3 py-2 text-sm text-fg hover:border-accent/50 disabled:opacity-50">{{ savingShell ? 'Saving…' : 'Save port' }}</button>
            <button :disabled="savingShell" @click="saveShell(false)"
              class="rounded-lg border border-line bg-bg px-3 py-2 text-sm text-muted hover:text-rose-400 hover:border-rose-400/50 disabled:opacity-50">Disable</button>
          </template>
        </div>
        <p v-if="shellMsg" class="w-full text-xs text-rose-400">{{ shellMsg }}</p>
        <p class="w-full text-[11px] text-faint">Owner-only. Enables interactive SSH console access for this host.</p>
      </div>

      <!-- connect-time auth hint -->
      <p class="text-[11px] text-faint">
        You'll choose your SSH user and either a host password or one of your keys when you connect.
        <router-link :to="{ name: 'ssh-keys' }" class="text-accent hover:underline">Manage your SSH keys</router-link>.
      </p>
    </template>
  </div>
</template>
