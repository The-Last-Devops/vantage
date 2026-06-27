<script setup>
// Presentational card listing the alert rules covering this host. Parent owns the
// data (rules) and the namespace query (nsq).
defineProps({
  rules: { type: Array, default: () => [] },
  nsq: { type: Object, default: () => ({}) },
})
</script>

<template>
  <!-- alert rules covering this host -->
  <div class="mb-4 rounded-xl border border-line bg-surface p-4">
    <div class="mb-2 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-faint">
      Alert rules <span class="rounded-full bg-surface2 px-2 py-0.5 text-[10px]">{{ rules.length }}</span>
    </div>
    <p v-if="!rules.length" class="text-xs text-faint">No alert rules cover this host. <RouterLink :to="{ name: 'alerts', query: nsq }" class="text-accent hover:underline">Add one</RouterLink>.</p>
    <div v-else class="flex flex-wrap gap-2">
      <RouterLink v-for="r in rules" :key="r.id" :to="{ name: 'alerts', query: { ...nsq, rule: r.id } }"
        class="inline-flex items-center gap-2 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-xs hover:border-accent/50">
        <span class="h-1.5 w-1.5 rounded-full" :class="r.firing === true ? 'bg-red-500' : r.firing === false ? 'bg-accent' : 'bg-faint'"></span>
        <span class="text-fg">{{ r.scope_kind === 'all_hosts' ? 'All hosts in namespace' : 'This host' }}</span>
        <span v-if="!r.enabled" class="text-faint">· off</span>
      </RouterLink>
    </div>
  </div>
</template>
