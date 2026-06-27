<script setup>
import UplotChart from './UplotChart.vue'

// One metric chart panel (card + header + UplotChart) for the host / container
// charts grids. Parent owns all data, chart config, and selection/zoom state;
// this is just the consistent panel chrome. `inlineSub` switches the two header
// layouts: host charts stack title/sub, the container leaf inlines the sub.
defineProps({
  chart: { type: Object, required: true }, // { title, sub, unit, series, area? }
  time: { type: Array, default: () => [] },
  headerTime: { type: String, default: '' },
  spanSeconds: { type: Number, default: 0 },
  syncKey: { type: String, default: '' },
  focusNames: { type: Array, default: null },
  selectedNames: { type: Array, default: () => [] },
  viewRange: { type: Array, default: null },
  inlineSub: { type: Boolean, default: false },
})
defineEmits(['legend-hover', 'legend-toggle', 'cursor-time', 'zoom'])
</script>

<template>
  <div class="rounded-xl border border-line bg-surface p-4">
    <div v-if="inlineSub" class="mb-2 flex items-start justify-between"><div class="text-sm font-medium text-fg">{{ chart.title }} <span class="text-xs text-faint">{{ chart.sub }}</span></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
    <div v-else class="mb-2 flex items-start justify-between"><div><div class="text-sm font-medium text-fg">{{ chart.title }}</div><div class="text-xs text-faint">{{ chart.sub }}</div></div><span class="tabular-nums text-xs text-faint">{{ headerTime }}</span></div>
    <UplotChart :time="time" :series="chart.series" :unit="chart.unit" :span-seconds="spanSeconds" :area="chart.area !== false" :sync-key="syncKey"
      :focus-names="focusNames" :selected-names="selectedNames" :view-range="viewRange"
      @legend-hover="$emit('legend-hover', $event)" @legend-toggle="$emit('legend-toggle', $event)" @cursor-time="$emit('cursor-time', $event)" @zoom="$emit('zoom', $event)" />
  </div>
</template>
