<script setup>
import UplotChart from './UplotChart.vue'

// Shared "fleet" grid: a chart per metric, each overlaying many entities (hosts,
// cluster nodes, or a docker host + its containers). The parent builds the chart
// defs and owns selection/zoom state; this component is just the consistent layout.
defineProps({
  charts: { type: Array, default: () => [] }, // [{ title, sub?, unit, series:[{name,color,data}] }]
  time: { type: Array, default: () => [] },
  spanSeconds: { type: Number, default: 0 },
  viewRange: { type: Array, default: null },
  focusNames: { type: Array, default: null },
  selectedNames: { type: Array, default: () => [] },
  syncKey: { type: String, default: 'fleet' },
})
defineEmits(['legend-hover', 'legend-toggle', 'zoom'])
</script>

<template>
  <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
    <div v-for="c in charts" :key="c.title" class="rounded-xl border border-line bg-surface p-4">
      <div class="mb-2 text-sm font-medium text-fg">{{ c.title }}<span v-if="c.sub" class="ml-2 text-xs text-faint">{{ c.sub }}</span></div>
      <UplotChart :time="time" :series="c.series" :unit="c.unit" :span-seconds="spanSeconds" :show-legend="false" :tooltip="true" :area="false" :sync-key="syncKey"
        :focus-names="focusNames" :selected-names="selectedNames" :view-range="viewRange"
        @legend-hover="$emit('legend-hover', $event)" @legend-toggle="$emit('legend-toggle', $event)" @zoom="$emit('zoom', $event)" />
    </div>
  </div>
</template>
