<script setup>
import { ref, reactive, computed, onMounted } from 'vue'
import { api } from '../lib/api'

const emit = defineEmits(['close'])
const HUB = location.origin

const TYPES = [
  { id: 'node', label: 'Node', desc: 'A single host — only this node’s own metrics. No container stats.' },
  { id: 'docker', label: 'Docker host', desc: 'Host metrics plus per-container stats (mounts the Docker socket).' },
  { id: 'k8s', label: 'Kubernetes', desc: 'Deploys a DaemonSet — every node in the cluster reports in.' },
]
const METHODS = {
  node: [
    { id: 'binary', label: 'Binary', snippet: (k) => `curl -fsSL ${HUB}/install.sh | \\\n  HUB_URL=${HUB} API_KEY=${k} sh` },
    { id: 'docker', label: 'Docker', snippet: (k) => `docker run -d --name last-agent \\\n  --network host --pid host \\\n  -e HUB_URL=${HUB} -e API_KEY=${k} \\\n  -v /proc:/host/proc:ro -v /sys:/host/sys:ro \\\n  ghcr.io/the-last-devops/last-monitor-agent:main` },
    { id: 'compose', label: 'Compose', snippet: (k) => `services:\n  last-agent:\n    image: ghcr.io/the-last-devops/last-monitor-agent:main\n    network_mode: host\n    pid: host\n    environment:\n      HUB_URL: ${HUB}\n      API_KEY: ${k}\n    volumes:\n      - /proc:/host/proc:ro\n      - /sys:/host/sys:ro\n    restart: unless-stopped` },
  ],
  docker: [
    { id: 'docker', label: 'Docker', snippet: (k) => `docker run -d --name last-agent \\\n  --network host --pid host \\\n  -e HUB_URL=${HUB} -e API_KEY=${k} -e AGENT_KIND=docker \\\n  -v /proc:/host/proc:ro -v /sys:/host/sys:ro \\\n  -v /var/run/docker.sock:/var/run/docker.sock:ro \\\n  ghcr.io/the-last-devops/last-monitor-agent:main` },
    { id: 'compose', label: 'Compose', snippet: (k) => `services:\n  last-agent:\n    image: ghcr.io/the-last-devops/last-monitor-agent:main\n    network_mode: host\n    pid: host\n    environment:\n      HUB_URL: ${HUB}\n      API_KEY: ${k}\n      AGENT_KIND: docker\n    volumes:\n      - /proc:/host/proc:ro\n      - /sys:/host/sys:ro\n      - /var/run/docker.sock:/var/run/docker.sock:ro\n    restart: unless-stopped` },
  ],
  k8s: [
    { id: 'kubectl', label: 'kubectl', snippet: (k) => `kubectl apply -f "${HUB}/k8s/agent.yaml?key=${k}&cluster=${state.cluster || 'my-cluster'}"` },
  ],
}

const namespaces = ref([])
const state = reactive({ type: 'node', method: 0, name: '', nsId: '', cluster: '', key: '', busy: false, error: '' })
const cfg = computed(() => TYPES.find((t) => t.id === state.type))
const methods = computed(() => METHODS[state.type])
const snippet = computed(() => (state.key ? methods.value[state.method].snippet(state.key) : ''))

onMounted(async () => {
  try {
    namespaces.value = await api.get('/api/namespaces')
    if (namespaces.value[0]) state.nsId = namespaces.value[0].id
  } catch { namespaces.value = [] }
})

function pickType(t) { state.type = t; state.method = 0; state.key = '' }

async function createKey() {
  if (!state.nsId) { state.error = 'Pick a namespace'; return }
  state.error = ''; state.busy = true
  try {
    // k8s uses the cluster name as the key's label; node/docker use Name.
    const label = state.type === 'k8s' ? state.cluster.trim() : state.name.trim()
    const name = (label || `${state.type}-key`).slice(0, 64)
    const res = await api.post(`/api/namespaces/${state.nsId}/keys`, { name })
    state.key = res.key
  } catch (e) {
    state.error = 'Could not create key (need editor role)'
  } finally { state.busy = false }
}

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
            <label class="block text-sm"><span class="text-muted">Namespace</span>
              <select v-model="state.nsId" class="mt-1.5 w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-fg outline-none focus:border-accent/50">
                <option v-for="n in namespaces" :key="n.id" :value="n.id">{{ n.name }}</option>
              </select>
            </label>
          </div>

          <!-- create / install -->
          <div v-if="!state.key">
            <button @click="createKey" :disabled="state.busy" class="w-full rounded-lg bg-accent px-4 py-2.5 font-semibold text-accentfg hover:opacity-90 disabled:opacity-50">{{ state.busy ? 'Creating…' : 'Create & show install command' }}</button>
            <p v-if="state.error" class="mt-2 text-sm text-red-500">{{ state.error }}</p>
          </div>
          <div v-else>
            <div class="mb-2 flex items-center justify-between">
              <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-xs">
                <button v-for="(m, i) in methods" :key="m.id" @click="state.method = i" class="rounded-md px-2.5 py-1" :class="i === state.method ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ m.label }}</button>
              </div>
              <button @click="copy" class="rounded-md border border-line bg-surface2 px-2 py-1 text-xs text-muted hover:text-accent">Copy</button>
            </div>
            <pre class="overflow-x-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ snippet }}</pre>
            <div class="mt-3 flex items-center gap-2 rounded-lg border border-line bg-surface2 px-3 py-2.5 text-xs text-muted">
              <span class="h-2 w-2 shrink-0 animate-pulse rounded-full bg-amber-500"></span>
              Waiting for first check-in… it will appear in the list automatically.
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
