<script setup>
// Generic bar for values with NO workspace threshold (monitor latency): <70% ok ·
// 70–90% warn · >90% critical of `max`. Host CPU/memory/disk use <Gauge>, which takes
// the workspace's thresholds — don't use this for those. Pairs a track bar with a mono value. Default slot overrides the printed value (e.g. "—" / "1.2").
import { computed } from 'vue'

const props = defineProps({
  value: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  width: { type: String, default: 'w-16' }, // track width utility
})
const pct = computed(() => Math.max(0, Math.min(100, (props.value / props.max) * 100)))
const barCls = computed(() => (pct.value > 90 ? 'bg-down' : pct.value >= 70 ? 'bg-warn' : 'bg-ok'))
</script>

<template>
  <span class="inline-flex items-center gap-2">
    <span class="inline-block h-1.5 overflow-hidden rounded bg-track align-middle" :class="width">
      <span class="block h-full rounded" :class="barCls" :style="{ width: pct + '%' }"></span>
    </span>
    <span class="font-mono text-sm tabular-nums text-fg"><slot>{{ Math.round(value) }}%</slot></span>
  </span>
</template>
