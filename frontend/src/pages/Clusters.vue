<script setup>
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'

const route = useRoute()
const router = useRouter()

const loaded = ref(false)
const rows = ref([])
const err = ref('')

// Only k8s clusters (systems of kind 'k8s-cluster' — the cluster-scoped agent).
// Filtered by the workspace selection (?ws), like other workspaced lists.
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const clusters = computed(() => {
  let list = rows.value.filter((s) => s.kind === 'k8s-cluster')
  if (selectedWs.value.length) list = list.filter((s) => selectedWs.value.includes(s.workspace))
  return [...list].sort((a, b) => (a.workspace || '').localeCompare(b.workspace || '') || a.name.localeCompare(b.name))
})
const online = (s) => s.last_seen && Date.now() / 1000 - new Date(s.last_seen).getTime() / 1000 < 90

async function load(first = false) {
  err.value = ''
  try { rows.value = await api.get('/api/systems') }
  catch (e) { err.value = `Failed to load (${e.status || '?'}).` }
  finally { if (first) loaded.value = true }
}
let timer = null
onMounted(() => { load(true); timer = setInterval(() => load(false), 8000) })
onBeforeUnmount(() => clearInterval(timer))

const open = (s) => router.push({ name: 'cluster', params: { id: s.id }, query: { ...(route.query.ws ? { ws: route.query.ws } : {}), name: s.name } })
</script>

<template>
  <AppShell title="Clusters">
    <PageLoader v-if="!loaded" />
    <template v-else>
      <p v-if="err" class="rounded-lg border border-down/40 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>
      <p v-else-if="!clusters.length" class="rounded-2xl border border-line bg-surface/50 p-10 text-center text-sm text-muted">
        No Kubernetes clusters yet. Deploy the cluster agent:
        <code class="mx-1 rounded bg-surface2 px-1.5 py-0.5 text-xs">kubectl apply -f "&lt;hub&gt;/pub/agent.yaml?key=…&amp;cluster=…"</code>
      </p>
      <div v-else class="grid gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
        <button v-for="c in clusters" :key="c.id" @click="open(c)"
          class="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-4 text-left transition-colors hover:border-accent/55">
          <div class="flex items-center gap-2.5">
            <span class="h-2.5 w-2.5 shrink-0 rounded-full" :class="online(c) ? 'bg-accent' : 'bg-down'" v-tip="online(c) ? 'online' : 'offline'"></span>
            <span class="truncate text-base font-semibold text-fg">{{ c.name }}</span>
            <span class="ml-auto shrink-0 rounded bg-surface2 px-2 py-0.5 text-[11px] text-muted">{{ c.workspace }}</span>
          </div>
          <div class="flex flex-wrap items-center gap-2 text-xs text-faint">
            <span>Kubernetes</span>
            <span v-if="c.k8s_version" class="rounded bg-surface2 px-1.5 py-0.5 font-mono text-fg" v-tip="'Kubernetes server version'">{{ c.k8s_version }}</span>
            <span v-else class="text-faint">version unknown</span>
            <span v-if="c.agent_version" class="ml-auto rounded bg-surface2 px-1.5 py-0.5 font-mono" v-tip="'Vantage agent version'">agent {{ c.agent_version }}</span>
          </div>
        </button>
      </div>
    </template>
  </AppShell>
</template>
