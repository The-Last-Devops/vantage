<script setup>
// Presentational "Down history" list of incidents for a service monitor.
// Incidents are computed by the parent (newest-first); formatting helpers are
// passed in so this component stays purely presentational.
defineProps({
  incidents: { type: Array, default: () => [] },
  evTime: { type: Function, required: true },
  durTxt: { type: Function, required: true },
})
</script>

<template>
  <div class="rounded-xl border border-line bg-surface p-4">
    <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Down history</div>
    <p v-if="!incidents.length" class="text-xs text-faint">No downtime in this range. 🎉</p>
    <ul v-else class="divide-y divide-line/60">
      <li v-for="(it, i) in incidents" :key="i" class="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5 text-sm">
        <span class="inline-flex items-center gap-1.5 font-medium" :class="it.ongoing ? 'text-red-500' : 'text-amber-400'">
          <span class="h-2 w-2 rounded-full" :class="it.ongoing ? 'bg-red-500' : 'bg-amber-400'"></span>
          {{ it.ongoing ? 'Down' : 'Resolved' }}
        </span>
        <span class="tabular-nums text-muted">{{ evTime(it.at) }}</span>
        <span class="text-faint">·</span>
        <span class="tabular-nums text-fg">{{ it.ongoing ? durTxt(Date.now() - it.start) + ' (ongoing)' : durTxt(it.end - it.start) }}</span>
        <span class="min-w-0 flex-1 truncate text-muted" v-tip="it.reason">{{ it.reason }}</span>
      </li>
    </ul>
  </div>
</template>
