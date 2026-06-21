<script setup>
import { ref, onMounted, onBeforeUnmount, watch, computed } from 'vue'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'
import { useUi } from '../stores/ui'

// props.series: [{ name, color, data: number[] }]; props.time: unix seconds[]
const props = defineProps({
  time: { type: Array, default: () => [] },
  series: { type: Array, default: () => [] },
  unit: { type: String, default: '' },
  height: { type: Number, default: 150 },
  syncKey: { type: String, default: '' }, // charts sharing a key sync their cursor
})

const ui = useUi()
const el = ref(null)
const hoverIdx = ref(null) // data index under cursor; null when not hovering
let u = null
let ro = null
let zoomed = false // user drag-zoomed → freeze the view; live data keeps appending off-screen

function cssVar(name) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v ? `rgb(${v})` : '#888'
}
function fmt(v) {
  if (v == null) return '—'
  if (props.unit === '%' || props.unit === '°C') return Math.round(v) + props.unit
  if (/B\/?s?/.test(props.unit)) {
    const us = ['B', 'K', 'M', 'G']; let i = 0; let n = v
    while (n >= 1024 && i < 3) { n /= 1024; i++ }
    return n.toFixed(n < 10 && i > 0 ? 1 : 0) + ' ' + us[i] + (props.unit.includes('/s') ? '/s' : '')
  }
  return (v < 10 ? v.toFixed(2) : v.toFixed(0)) + props.unit
}

const uData = computed(() => [props.time, ...props.series.map((s) => s.data)])
// index used for the legend: cursor when hovering, else the latest sample
const valueIdx = computed(() => (hoverIdx.value != null ? hoverIdx.value : props.time.length - 1))
const legend = computed(() =>
  props.series.map((s) => ({ name: s.name, color: s.color, value: fmt(s.data[valueIdx.value]) })),
)
const cursorTime = computed(() => {
  const t = props.time[valueIdx.value]
  if (!t) return ''
  const d = new Date(t * 1000)
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
})

function opts() {
  const axis = cssVar('--muted'), grid = cssVar('--line')
  return {
    width: el.value?.clientWidth || 400,
    height: props.height,
    padding: [10, 8, 0, 0],
    legend: { show: false }, // we render our own
    cursor: {
      points: { size: 7 },
      focus: { prox: 30 },
      drag: { x: true, y: false }, // drag to select/zoom a time range; dblclick resets
      sync: props.syncKey ? { key: props.syncKey, scales: ['x', null] } : undefined,
    },
    scales: { x: { time: true } },
    series: [
      {},
      ...props.series.map((s) => ({ label: s.name, stroke: s.color, width: 1.6, fill: s.color + '22', points: { show: false } })),
    ],
    axes: [
      { stroke: axis, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid }, font: '11px ui-monospace, monospace' },
      { stroke: axis, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid }, font: '11px ui-monospace, monospace', size: 46, values: (_u, vals) => vals.map(fmt) },
    ],
    hooks: {
      setCursor: [(up) => { hoverIdx.value = up.cursor.idx }],
      setScale: [(up, key) => {
        if (key !== 'x') return
        const t = up.data[0]
        if (!t || !t.length) { zoomed = false; return }
        // zoomed if the visible x-range is narrower than the full data extent
        zoomed = up.scales.x.min > t[0] + 1 || up.scales.x.max < t[t.length - 1] - 1
      }],
    },
  }
}

function build() {
  if (u) { u.destroy(); u = null }
  if (!el.value) return
  u = new uPlot(opts(), uData.value, el.value)
}

onMounted(() => {
  build()
  ro = new ResizeObserver(() => u && u.setSize({ width: el.value.clientWidth, height: props.height }))
  ro.observe(el.value)
})
onBeforeUnmount(() => { ro && ro.disconnect(); u && u.destroy() })
// follow the latest when not zoomed; keep the frozen view (append off-screen) when zoomed
watch(uData, (d) => { if (u) u.setData(d, !zoomed) })
watch(() => ui.light, () => build())
</script>

<template>
  <div>
    <div ref="el" class="w-full"></div>
    <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs">
      <span v-for="s in legend" :key="s.name" class="flex items-center gap-1.5">
        <span class="h-2 w-2 rounded-full" :style="{ background: s.color }"></span>
        <span class="text-muted">{{ s.name }}</span>
        <span class="tabular-nums text-fg">{{ s.value }}</span>
      </span>
      <span class="ml-auto tabular-nums text-faint">{{ hoverIdx != null ? cursorTime : 'now' }}</span>
    </div>
  </div>
</template>

<style>
.uplot { font-family: ui-monospace, monospace; }
</style>
