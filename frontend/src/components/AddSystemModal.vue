<script setup>
import { ref, reactive, computed, onMounted, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../lib/api'

const route = useRoute()
// the workspace selected in the sidebar (?ws=), if exactly one is active
function selectedWsName() {
  const sel = (route.query.ws || '').split(',').filter(Boolean)
  return sel.length === 1 ? sel[0] : null
}

const emit = defineEmits(['close'])
const HUB = location.origin

const TYPES = [
  { id: 'node', label: 'Node', desc: 'A single host — only this node’s own metrics. No container stats.' },
  { id: 'docker', label: 'Docker host', desc: 'Host metrics plus per-container stats (mounts the Docker socket).' },
  { id: 'k8s', label: 'Kubernetes', desc: 'Deploys a DaemonSet — every node in the cluster reports in.' },
]
const METHODS = {
  node: [
    { id: 'binary', label: 'Binary', snippet: (k) => `curl -fsSL ${HUB}/pub/install.sh | \\\n  HUB_URL=${HUB} API_KEY=${k} sh` },
    { id: 'docker', label: 'Docker', snippet: (k) => `docker run -d --name vantage-agent \\\n  --network host --pid host \\\n  -e HUB_URL=${HUB} -e API_KEY=${k} \\\n  -v /proc:/host/proc:ro -v /sys:/host/sys:ro \\\n  ghcr.io/the-last-devops/vantage-agent:main` },
    { id: 'compose', label: 'Compose', snippet: (k) => `services:\n  vantage-agent:\n    image: ghcr.io/the-last-devops/vantage-agent:main\n    network_mode: host\n    pid: host\n    environment:\n      HUB_URL: ${HUB}\n      API_KEY: ${k}\n    volumes:\n      - /proc:/host/proc:ro\n      - /sys:/host/sys:ro\n    restart: unless-stopped` },
  ],
  docker: [
    { id: 'docker', label: 'Docker', snippet: (k) => `docker run -d --name vantage-agent \\\n  --network host --pid host \\\n  -e HUB_URL=${HUB} -e API_KEY=${k} -e AGENT_KIND=docker \\\n  -v /proc:/host/proc:ro -v /sys:/host/sys:ro \\\n  -v /var/run/docker.sock:/var/run/docker.sock:ro \\\n  ghcr.io/the-last-devops/vantage-agent:main` },
    { id: 'compose', label: 'Compose', snippet: (k) => `services:\n  vantage-agent:\n    image: ghcr.io/the-last-devops/vantage-agent:main\n    network_mode: host\n    pid: host\n    environment:\n      HUB_URL: ${HUB}\n      API_KEY: ${k}\n      AGENT_KIND: docker\n    volumes:\n      - /proc:/host/proc:ro\n      - /sys:/host/sys:ro\n      - /var/run/docker.sock:/var/run/docker.sock:ro\n    restart: unless-stopped` },
  ],
  k8s: [
    { id: 'kubectl', label: 'kubectl', snippet: (k) => `kubectl apply -f "${HUB}/pub/agent.yaml?key=${k}&cluster=${state.cluster || 'my-cluster'}${state.tag && state.tag !== 'latest' ? '&tag=' + state.tag : ''}"` },
  ],
}

const workspaces = ref([])
const TAGS = ['latest', 'main']
const state = reactive({ type: 'node', method: 0, name: '', wsId: '', cluster: '', tag: 'latest', key: '', keyId: '', connected: [], busy: false, error: '' })
const cfg = computed(() => TYPES.find((t) => t.id === state.type))
const methods = computed(() => METHODS[state.type])
const snippet = computed(() => (state.key ? methods.value[state.method].snippet(state.key) : ''))

onMounted(async () => {
  try {
    workspaces.value = await api.get('/api/workspaces')
    const match = workspaces.value.find((n) => n.name === selectedWsName())
    state.wsId = (match || workspaces.value[0])?.id || ''
  } catch { workspaces.value = [] }
})

function pickType(t) { state.type = t; state.method = 0; state.key = ''; stopPolling(); state.connected = [] }

async function createKey() {
  if (!state.wsId) { state.error = 'Pick a workspace'; return }
  state.error = ''; state.busy = true
  try {
    // k8s uses the cluster name as the key's label; node/docker use Name.
    const label = state.type === 'k8s' ? state.cluster.trim() : state.name.trim()
    const name = (label || `${state.type}-key`).slice(0, 64)
    const res = await api.post(`/api/workspaces/${state.wsId}/keys`, { name })
    state.key = res.key
    state.keyId = res.id
    startPolling()
  } catch (e) {
    state.error = 'Could not create key (need editor role)'
  } finally { state.busy = false }
}

// Poll the new key for enrolled hosts so the "waiting" box flips to success the
// moment the agent checks in.
let pollTimer = null
function startPolling() {
  stopPolling()
  const tick = async () => {
    try {
      const r = await api.get(`/api/keys/${state.keyId}/systems`)
      state.connected = r.systems || []
    } catch {}
  }
  tick()
  pollTimer = setInterval(tick, 3000)
}
function stopPolling() { if (pollTimer) { clearInterval(pollTimer); pollTimer = null } }
onBeforeUnmount(stopPolling)

function copy(ev) {
  navigator.clipboard?.writeText(snippet.value)
  const b = ev.target; const o = b.textContent; b.textContent = 'Copied'; setTimeout(() => (b.textContent = o), 1200)
}
</script>

<template>
  <div class="fixed inset-0 z-50">
    <div class="absolute inset-0 bg-black/60" @click="emit('close')"></div>
    <div class="absolute inset-0 flex items-start justify-center overflow-y-auto p-4 sm:p-8">
      <div class="relative w-full max-w-lg rounded-xl border border-line bg-surface shadow-2xl">
        <div class="flex items-start justify-between border-b border-line p-5">
          <div>
            <h2 class="text-base font-semibold text-fg">Add system</h2>
            <p class="mt-0.5 text-xs text-muted">{{ cfg.desc }}</p>
          </div>
          <button @click="emit('close')" class="text-muted hover:text-fg"><svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12"/></svg></button>
        </div>

        <div class="space-y-5 p-5">
          <!-- type -->
          <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-sm">
            <button v-for="t in TYPES" :key="t.id" @click="pickType(t.id)" class="flex-1 rounded-md px-2.5 py-1.5" :class="state.type === t.id ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ t.label }}</button>
          </div>

          <!-- details -->
          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label v-if="state.type !== 'k8s'" class="block text-sm"><span class="text-muted">Name</span>
              <input v-model="state.name" placeholder="e.g. web-03" class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-fg outline-none focus:border-accent/50" />
            </label>
            <label v-if="state.type === 'k8s'" class="block text-sm"><span class="text-muted">Cluster name</span>
              <input v-model="state.cluster" placeholder="e.g. prod-cluster" class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-fg outline-none focus:border-accent/50" />
            </label>
            <label class="block text-sm"><span class="text-muted">Workspace</span>
              <UiSelect v-model="state.wsId" block class="mt-1.5" :options="workspaces.map((n) => ({ value: n.id, label: n.name }))" />
            </label>
          </div>

          <!-- image tag (k8s only): pick which agent image the manifest runs -->
          <label v-if="state.type === 'k8s'" class="block text-sm"><span class="text-muted">Image tag</span>
            <UiSelect v-model="state.tag" block class="mt-1.5" :options="TAGS" />
            <span class="mt-1 block text-xs text-faint"><span class="font-mono">latest</span> = newest release; <span class="font-mono">main</span> = rolling build. Updates are driven by re-applying with a new tag (or your GitOps tool).</span>
          </label>

          <!-- create / install -->
          <div v-if="!state.key">
            <button @click="createKey" :disabled="state.busy" class="w-full rounded-lg bg-accent px-4 py-2.5 font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ state.busy ? 'Creating…' : 'Create & show install command' }}</button>
            <p v-if="state.error" class="mt-2 text-sm text-down">{{ state.error }}</p>
          </div>
          <div v-else>
            <div class="mb-2 flex items-center justify-between">
              <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-xs">
                <button v-for="(m, i) in methods" :key="m.id" @click="state.method = i" class="rounded-md px-2.5 py-1" :class="i === state.method ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ m.label }}</button>
              </div>
              <button @click="copy" class="rounded-md border border-line bg-surface2 px-2 py-1 text-xs text-muted hover:text-accent">Copy</button>
            </div>
            <pre class="overflow-x-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ snippet }}</pre>
            <div v-if="!state.connected.length" class="mt-3 flex items-center gap-2 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-xs text-muted">
              <span class="h-2 w-2 shrink-0 animate-pulse rounded-full bg-warn"></span>
              Waiting for first check-in… it will appear in the list automatically.
            </div>
            <div v-else class="mt-3 rounded-lg border border-accent/40 bg-accent/10 px-3 py-2.5 text-xs text-accent">
              <div class="flex items-center gap-2 font-medium">
                <svg class="h-3.5 w-3.5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><path d="M20 6 9 17l-5-5"/></svg>
                {{ state.connected.length }} node{{ state.connected.length > 1 ? 's' : '' }} connected
              </div>
              <div class="mt-1 truncate text-muted">{{ state.connected.slice(0, 4).join(', ') }}<span v-if="state.connected.length > 4"> +{{ state.connected.length - 4 }}</span></div>
            </div>
          </div>
        </div>

        <div class="flex justify-end gap-2 border-t border-line p-4">
          <button @click="emit('close')" class="rounded-lg border border-line px-4 py-2 text-sm text-muted hover:text-fg">{{ state.key ? 'Done' : 'Cancel' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>
