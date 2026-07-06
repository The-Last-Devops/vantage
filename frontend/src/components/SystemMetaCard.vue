<script setup>
// Presentational metadata bar for a host or container. The parent owns the data
// (meta) and the route; this only renders it. Every field links to the fleet
// filtered by that value.
const props = defineProps({
  meta: { type: Object, required: true },
  type: { type: String, required: true },
  id: { required: true },
})
const TYPE_LABEL = { node: 'Node', host: 'Host', docker: 'Docker', k8s: 'Kubernetes', container: 'Container' }
// Total bytes → human size (RAM/disk capacity in the metadata bar).
const fmtBytes = (v) => {
  if (v == null || v <= 0) return null
  const u = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0; let n = v
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return `${n.toFixed(n < 10 && i > 1 ? 1 : 0)} ${u[i]}`
}
</script>

<template>
  <!-- node metadata — every field links to the fleet filtered by that value -->
  <div v-if="type !== 'container'" class="mb-4 flex flex-wrap items-center gap-x-6 gap-y-1.5 rounded-xl border border-line bg-surface px-4 py-2.5 text-xs">
    <span><span class="text-faint">Type</span> <RouterLink :to="{ path: '/', query: { q: `kind:${meta.kind}` } }" class="text-fg hover:text-accent">{{ TYPE_LABEL[meta.kind] || meta.kind }}</RouterLink></span>
    <span v-if="meta.cluster"><span class="text-faint">Cluster</span> <RouterLink :to="{ path: '/', query: { q: `cluster:${meta.cluster}` } }" class="text-fg hover:text-accent">{{ meta.cluster }}</RouterLink></span>
    <span><span class="text-faint">Workspace</span> <RouterLink :to="{ path: '/', query: { q: `ws:${meta.workspace}` } }" class="text-fg hover:text-accent">{{ meta.workspace }}</RouterLink></span>
    <span v-if="meta.hostname"><span class="text-faint">Hostname</span> <RouterLink :to="{ path: '/', query: { q: meta.hostname } }" class="text-fg hover:text-accent">{{ meta.hostname }}</RouterLink></span>
    <span v-if="meta.cpu_model"><span class="text-faint">CPU</span> <span class="text-fg">{{ meta.cpu_model }}<template v-if="meta.cpu_cores"> · {{ meta.cpu_cores }} cores</template></span></span>
    <span v-if="fmtBytes(meta.mem_total)"><span class="text-faint">RAM</span> <span class="text-fg">{{ fmtBytes(meta.mem_total) }}</span></span>
    <span v-if="fmtBytes(meta.disk_total)"><span class="text-faint">Disk</span> <span class="text-fg">{{ fmtBytes(meta.disk_total) }}</span></span>
    <span v-if="meta.kernel"><span class="text-faint">Kernel</span> <RouterLink :to="{ path: '/', query: { q: `kernel:${meta.kernel}` } }" class="text-fg hover:text-accent">{{ meta.kernel }}</RouterLink></span>
    <span v-if="meta.agent_version"><span class="text-faint">Agent</span> <RouterLink :to="{ path: '/', query: { q: `agent:${meta.agent_version}` } }" class="text-fg hover:text-accent">v{{ meta.agent_version }}</RouterLink></span>
  </div>

  <!-- container metadata: context about its host -->
  <div v-if="type === 'container'" class="mb-4 flex flex-wrap items-center gap-x-6 gap-y-1.5 rounded-xl border border-line bg-surface px-4 py-2.5 text-xs">
    <span><span class="text-faint">Type</span> <span class="text-fg">Container</span></span>
    <span><span class="text-faint">Host</span> <RouterLink :to="`/system/${id}?type=docker&name=${encodeURIComponent(meta.name)}`" class="text-fg hover:text-accent">{{ meta.name }}</RouterLink></span>
    <span><span class="text-faint">Workspace</span> <RouterLink :to="{ path: '/', query: { q: `ws:${meta.workspace}` } }" class="text-fg hover:text-accent">{{ meta.workspace }}</RouterLink></span>
    <span v-if="meta.cpu_model"><span class="text-faint">Host CPU</span> <span class="text-fg">{{ meta.cpu_model }}<template v-if="meta.cpu_cores"> · {{ meta.cpu_cores }} cores</template></span></span>
    <span v-if="meta.kernel"><span class="text-faint">Host kernel</span> <span class="text-fg">{{ meta.kernel }}</span></span>
    <RouterLink :to="`/system/${id}?type=containers&name=${encodeURIComponent(meta.name)}`" class="ml-auto text-accent hover:underline">All containers ›</RouterLink>
  </div>
</template>
