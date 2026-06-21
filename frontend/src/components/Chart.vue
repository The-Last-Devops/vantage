<script setup>
import { computed } from 'vue'
// series: [{ name, color, data: number[] }]; lightweight SVG line chart + legend.
const props = defineProps({ series: { type: Array, default: () => [] }, unit: { type: String, default: '' } })
const W = 520, H = 120
const max = computed(() => {
  const vals = props.series.flatMap((s) => s.data).filter((v) => v != null)
  return Math.max(1, ...vals) * 1.1
})
function line(data) {
  const n = data.length
  if (!n) return ''
  const sx = W / Math.max(1, n - 1)
  return data.map((v, i) => `${(i * sx).toFixed(1)},${(H - ((v || 0) / max.value) * H).toFixed(1)}`).join(' ')
}
const lastVal = (d) => { for (let i = d.length - 1; i >= 0; i--) if (d[i] != null) return d[i]; return null }
function fmt(v) {
  if (v == null) return '—'
  if (props.unit === '%' || props.unit === '°C') return Math.round(v) + props.unit
  if (v >= 1e6) return (v / 1e6).toFixed(1) + ' M' + props.unit
  if (v >= 1e3) return (v / 1e3).toFixed(1) + ' K' + props.unit
  return v.toFixed(0) + props.unit
}
</script>

<template>
  <div>
    <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="h-28 w-full">
      <template v-for="(s, i) in series" :key="s.name">
        <polygon :points="`0,${H} ${line(s.data)} ${W},${H}`" :fill="s.color" fill-opacity="0.12" />
        <polyline :points="line(s.data)" fill="none" :stroke="s.color" stroke-width="1.5" />
      </template>
    </svg>
    <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
      <span v-for="s in series" :key="s.name" class="flex items-center gap-1.5">
        <span class="h-2 w-2 rounded-sm" :style="{ background: s.color }"></span>
        <span class="text-muted">{{ s.name }}</span>
        <span class="tabular-nums text-fg">{{ fmt(lastVal(s.data)) }}</span>
      </span>
    </div>
  </div>
</template>
