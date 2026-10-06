<script setup>
// Inline history line for one metric of one host — the table-cell chart from issue #1.
// Plain SVG, no uPlot: the Hosts table draws two of these per row and 70+ rows, and a
// uPlot instance each would cost more than the page it sits in. No axes: the gauge next
// to it carries the current number; this only shows the shape of the last range.
// Hover → the value and time at the cursor, as a small tooltip inside the cell.
import { computed, ref } from 'vue'

const props = defineProps({
  data: { type: Array, default: () => [] }, // number|null per point (null = gap)
  time: { type: Array, default: () => [] }, // unix seconds, same length as data
  max: { type: Number, default: null }, // fixed ceiling (100 for %); null = auto-fit
  unit: { type: String, default: '%' },
  width: { type: Number, default: 96 },
  height: { type: Number, default: 22 },
  color: { type: String, default: 'currentColor' },
})

const W = computed(() => props.width), H = computed(() => props.height)
const PAD = 1.5
// Y ceiling: fixed for percentages (an idle 12% host reads flat, not volatile), else the
// data's own max so an absolute metric still fills the strip.
const ceil = computed(() => {
  if (props.max != null) return props.max
  let m = 0
  for (const v of props.data) if (v != null && v > m) m = v
  return m || 1
})
const n = computed(() => props.data.length)
const xOf = (i) => (n.value < 2 ? W.value / 2 : PAD + (i / (n.value - 1)) * (W.value - 2 * PAD))
const yOf = (v) => H.value - PAD - Math.max(0, Math.min(1, v / ceil.value)) * (H.value - 2 * PAD)
// One path per run of non-null points, so an agent gap shows as a hole, not a bridge.
const paths = computed(() => {
  const out = []
  let cur = ''
  props.data.forEach((v, i) => {
    if (v == null) { if (cur) out.push(cur); cur = ''; return }
    cur += `${cur ? 'L' : 'M'}${xOf(i).toFixed(1)},${yOf(v).toFixed(1)}`
  })
  if (cur) out.push(cur)
  return out
})
const hasData = computed(() => paths.value.length > 0)

const hover = ref(null) // index under cursor
function onMove(e) {
  if (!n.value) return
  const r = e.currentTarget.getBoundingClientRect()
  const x = ((e.clientX - r.left) / r.width) * W.value
  const i = Math.round(((x - PAD) / (W.value - 2 * PAD)) * (n.value - 1))
  hover.value = Math.max(0, Math.min(n.value - 1, i))
}
const fmtVal = (v) => (v == null ? '—' : props.unit === '%' ? `${Math.round(v)}%` : `${v}${props.unit}`)
const fmtTs = (ts) => (ts ? new Date(ts * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }) : '')
const tip = computed(() => (hover.value == null ? null : { v: fmtVal(props.data[hover.value]), t: fmtTs(props.time[hover.value]), x: xOf(hover.value), y: props.data[hover.value] == null ? null : yOf(props.data[hover.value]) }))
</script>

<template>
  <span class="relative inline-block align-middle" :style="{ width: W + 'px', height: H + 'px' }" @mousemove="onMove" @mouseleave="hover = null">
    <svg v-if="hasData" :viewBox="`0 0 ${W} ${H}`" :width="W" :height="H" class="block overflow-visible" :style="{ color }">
      <path v-for="(d, i) in paths" :key="i" :d="d" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round" stroke-linecap="round" />
      <template v-if="tip">
        <line :x1="tip.x" :x2="tip.x" :y1="0" :y2="H" stroke="currentColor" stroke-opacity="0.35" stroke-width="1" />
        <circle v-if="tip.y != null" :cx="tip.x" :cy="tip.y" r="2" fill="currentColor" />
      </template>
    </svg>
    <span v-else class="block h-full w-full rounded bg-line/40"></span>
    <span v-if="tip" class="pointer-events-none absolute -top-7 left-1/2 z-10 -translate-x-1/2 whitespace-nowrap rounded border border-line bg-surface2 px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-fg shadow">
      {{ tip.v }} <span class="text-faint">· {{ tip.t }}</span>
    </span>
  </span>
</template>
