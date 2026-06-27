<script setup>
// Presentational time-range picker for the charts views. Parent owns the active
// range + resolution and reacts to set-range.
defineProps({
  ranges: { type: Array, default: () => [] }, // [[range, res], ...]
  range: { type: String, default: '30m' },
  resOf: { type: String, default: '1m' },
  live: { type: Boolean, default: false },
})
defineEmits(['set-range'])
</script>

<template>
  <!-- range (charts views) -->
  <div class="mb-4 flex flex-wrap items-center gap-2">
    <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-sm">
      <button v-for="[rr] in ranges" :key="rr" @click="$emit('set-range', rr)" class="rounded-md px-3 py-1" :class="range === rr ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ rr }}</button>
    </div>
    <span class="text-xs text-muted">Resolution <span class="rounded bg-surface2 px-1.5 py-0.5 text-fg">{{ resOf }}</span></span>
    <span v-if="live" class="ml-auto flex items-center gap-1.5 text-xs text-accent"><span class="h-1.5 w-1.5 animate-pulse rounded-full bg-accent"></span>Live</span>
    <span v-else class="ml-auto text-xs text-faint">auto-refresh 5s</span>
  </div>
</template>
