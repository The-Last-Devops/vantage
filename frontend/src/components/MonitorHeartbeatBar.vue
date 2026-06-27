<script setup>
// Presentational up/down status strip for a service monitor's heartbeats.
// `up` is the per-bucket array (null = no data, >=1 = up, else down).
defineProps({
  up: { type: Array, default: () => [] },
})
</script>

<template>
  <div class="rounded-xl border border-line bg-surface p-4">
    <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Status</div>
    <div v-if="up.length" class="flex h-7 gap-px overflow-hidden rounded">
      <div v-for="(u, i) in up" :key="i" class="flex-1"
        :class="u == null ? 'bg-line' : u >= 1 ? 'bg-accent' : 'bg-red-500'"
        v-tip="u == null ? 'no data' : u >= 1 ? 'up' : 'down'"></div>
    </div>
    <p v-else class="text-xs text-faint">No heartbeats in this range yet.</p>
  </div>
</template>
