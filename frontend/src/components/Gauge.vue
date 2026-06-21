<script setup>
defineProps({ v: { default: null } })
const cls = (x) => (x >= 90 ? 'bg-red-500' : x >= 70 ? 'bg-amber-500' : 'bg-accent')
const tcls = (x) => (x >= 90 ? 'text-red-500' : x >= 70 ? 'text-amber-500' : 'text-fg')
</script>

<template>
  <span v-if="v == null" class="text-faint">—</span>
  <div v-else class="flex items-center gap-2">
    <div class="h-1.5 w-16 overflow-hidden rounded bg-line"><div class="h-full" :class="cls(v)" :style="{ width: v + '%' }"></div></div>
    <span class="tabular-nums" :class="tcls(v)">{{ v }}%</span>
  </div>
</template>
