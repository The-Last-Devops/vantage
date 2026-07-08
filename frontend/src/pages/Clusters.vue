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
const sums = ref({}) // system id -> kube/summary
const err = ref('')

// Only k8s clusters (kind 'k8s-cluster'), filtered by the workspace selection (?ws).
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const clusters = computed(() => {
  let list = rows.value.filter((s) => s.kind === 'k8s-cluster')
  if (selectedWs.value.length) list = list.filter((s) => selectedWs.value.includes(s.workspace))
  return [...list].sort((a, b) => (a.workspace || '').localeCompare(b.workspace || '') || a.name.localeCompare(b.name))
})
const online = (s) => s.last_seen && Date.now() / 1000 - new Date(s.last_seen).getTime() / 1000 < 90

// roll-up across the visible clusters
const roll = computed(() => {
  const acc = { on: 0, nodes: 0, pods: 0, cpu: 0, mem: 0 }
  for (const c of clusters.value) {
    if (online(c)) acc.on++
    const s = sums.value[c.id]
    if (s) { acc.nodes += s.nodes; acc.pods += s.pods_running; acc.cpu += s.cpu_millicores; acc.mem += s.mem_bytes }
  }
  return acc
})

async function load(first = false) {
  err.value = ''
  try {
    rows.value = await api.get('/api/systems')
    // fetch each cluster's summary in parallel (few clusters)
    const cl = rows.value.filter((s) => s.kind === 'k8s-cluster')
    const got = await Promise.all(cl.map((c) => api.get(`/api/systems/${c.id}/kube/summary`).then((s) => [c.id, s]).catch(() => null)))
    const m = {}
    for (const g of got) if (g) m[g[0]] = g[1]
    sums.value = m
  } catch (e) { err.value = `Failed to load (${e.status || '?'}).` }
  finally { if (first) loaded.value = true }
}
let timer = null
onMounted(() => { load(true); timer = setInterval(() => load(false), 8000) })
onBeforeUnmount(() => clearInterval(timer))

const open = (s) => router.push({ name: 'cluster', params: { id: s.id }, query: { ...(route.query.ws ? { ws: route.query.ws } : {}), name: s.name } })

function fmtBytes(b) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0, n = Number(b) || 0
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${u[i]}`
}
const kCores = (mc) => (mc / 1000).toFixed(mc >= 100000 ? 0 : 1)
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
      <div v-else class="space-y-5">
        <!-- roll-up across all visible clusters -->
        <div class="grid grid-cols-2 gap-px overflow-hidden rounded-2xl border border-line bg-line sm:grid-cols-5">
          <div class="bg-surface px-4 py-2.5"><div class="text-[10px] font-bold uppercase tracking-wide text-faint">Clusters</div><div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ clusters.length }}<span class="text-xs font-semibold text-faint"> / {{ roll.on }} online</span></div></div>
          <div class="bg-surface px-4 py-2.5"><div class="text-[10px] font-bold uppercase tracking-wide text-faint">Nodes</div><div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ roll.nodes }}</div></div>
          <div class="bg-surface px-4 py-2.5"><div class="text-[10px] font-bold uppercase tracking-wide text-faint">Pods running</div><div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ roll.pods }}</div></div>
          <div class="bg-surface px-4 py-2.5"><div class="text-[10px] font-bold uppercase tracking-wide text-faint">CPU used</div><div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-accent">{{ kCores(roll.cpu) }}<span class="text-xs font-semibold text-faint"> cores</span></div></div>
          <div class="bg-surface px-4 py-2.5"><div class="text-[10px] font-bold uppercase tracking-wide text-faint">Memory used</div><div class="mt-0.5 font-mono text-xl font-extrabold tabular-nums text-fg">{{ fmtBytes(roll.mem) }}</div></div>
        </div>

        <!-- cluster cards -->
        <div class="grid gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
          <button v-for="c in clusters" :key="c.id" @click="open(c)"
            class="flex flex-col gap-3.5 rounded-2xl border border-line bg-surface p-4 text-left transition-colors hover:border-accent/55">
            <div class="flex items-center gap-2.5">
              <span class="h-2.5 w-2.5 shrink-0 rounded-full" :class="online(c) ? 'bg-accent' : 'bg-down'" v-tip="online(c) ? 'online' : 'offline'"></span>
              <span class="truncate text-base font-semibold text-fg">{{ c.name }}</span>
              <span class="ml-auto shrink-0 rounded-full bg-surface2 px-2 py-0.5 text-[11px] text-muted">{{ c.workspace }}</span>
            </div>
            <div class="flex flex-wrap items-center gap-2 text-xs">
              <span v-if="c.k8s_version" class="rounded-full border border-line2 bg-surface2 px-2 py-0.5 font-mono text-fg" v-tip="'Kubernetes server version'">{{ c.k8s_version }}</span>
              <span v-else class="text-faint italic">version unknown</span>
              <span v-if="c.agent_version" class="rounded-full border border-line2 bg-surface2 px-2 py-0.5 font-mono text-faint" v-tip="'Vantage agent version'">agent {{ c.agent_version }}</span>
            </div>
            <div class="grid grid-cols-4 gap-2 border-t border-line pt-3">
              <div><div class="text-[9px] font-bold uppercase tracking-wide text-cap">Nodes</div><div class="mt-0.5 font-mono text-base font-bold tabular-nums text-fg">{{ sums[c.id]?.nodes ?? '—' }}</div></div>
              <div><div class="text-[9px] font-bold uppercase tracking-wide text-cap">Pods</div><div class="mt-0.5 font-mono text-base font-bold tabular-nums text-fg">{{ sums[c.id]?.pods_running ?? '—' }}</div></div>
              <div><div class="text-[9px] font-bold uppercase tracking-wide text-cap">CPU</div><div class="mt-0.5 font-mono text-base font-bold tabular-nums text-fg">{{ sums[c.id] ? kCores(sums[c.id].cpu_millicores) : '—' }}<small v-if="sums[c.id]" class="text-[10px] font-semibold text-faint"> c</small></div></div>
              <div><div class="text-[9px] font-bold uppercase tracking-wide text-cap">Memory</div><div class="mt-0.5 font-mono text-base font-bold tabular-nums text-fg">{{ sums[c.id] ? fmtBytes(sums[c.id].mem_bytes) : '—' }}</div></div>
            </div>
            <div class="flex items-center gap-2 text-[11px] text-faint">
              <span>Kubernetes cluster</span>
              <span v-if="sums[c.id]?.restarts" :class="sums[c.id].restarts > 50 ? 'text-warn' : ''">· {{ sums[c.id].restarts }} restarts</span>
              <span class="ml-auto text-cap">›</span>
            </div>
          </button>
        </div>
      </div>
    </template>
  </AppShell>
</template>
