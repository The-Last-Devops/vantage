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
  spanSeconds: { type: Number, default: 0 }, // >0 → x-axis spans the full [now-span, now]
  showLegend: { type: Boolean, default: true },
  legendValuesAlways: { type: Boolean, default: true }, // false → show values only on hover
  area: { type: Boolean, default: true }, // false → lines only (cleaner for many-host overlay)
  spanGaps: { type: Boolean, default: false }, // true → connect across missing buckets
  // legend interaction (controlled by parent): focusNames = series to show (others
  // hidden); null = show all. selectedNames = pinned series (for legend styling).
  focusNames: { type: Array, default: null },
  selectedNames: { type: Array, default: () => [] },
})
const emit = defineEmits(['legend-hover', 'legend-toggle'])
const isSel = (n) => props.selectedNames.includes(n)
const isDim = (n) => props.focusNames != null && !props.focusNames.includes(n)

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

// When spanSeconds is set, pad with null boundary points at [now-span] and [now]
// so the x-axis always covers the whole selected window (blank where no data),
// instead of uPlot shrinking to the data extent. `prepend` = points added at the
// front, used to map uPlot's cursor index back to the unpadded series.
const padded = computed(() => {
  const time = props.time.slice()
  const cols = props.series.map((s) => s.data.slice())
  let prepend = 0
  if (props.spanSeconds > 0) {
    const to = Math.floor(Date.now() / 1000)
    const from = to - props.spanSeconds
    if (!time.length || time[0] > from) { time.unshift(from); cols.forEach((c) => c.unshift(null)); prepend = 1 }
    if (!time.length || time[time.length - 1] < to) { time.push(to); cols.forEach((c) => c.push(null)) }
  }
  return { data: [time, ...cols], prepend }
})
const uData = computed(() => padded.value.data)
// index used for the legend: cursor when hovering (mapped past any prepended
// boundary point), else the latest real sample
const valueIdx = computed(() => {
  if (hoverIdx.value == null) return props.time.length - 1
  return Math.max(0, Math.min(props.time.length - 1, hoverIdx.value - padded.value.prepend))
})
const legend = computed(() =>
  props.series.map((s) => ({
    name: s.name,
    color: s.color,
    value: props.legendValuesAlways || hoverIdx.value != null ? fmt(s.data[valueIdx.value]) : '',
  })),
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
      ...props.series.map((s) => ({
        label: s.name,
        stroke: s.color,
        width: 1.6,
        fill: props.area ? s.color + '22' : undefined,
        spanGaps: props.spanGaps,
        points: { show: false },
      })),
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

// show only the focused series across the chart (others hidden); null = all
function applyFocus() {
  if (!u) return
  const f = props.focusNames
  props.series.forEach((s, i) => u.setSeries(i + 1, { show: f == null || f.includes(s.name) }))
}

function build() {
  if (u) { u.destroy(); u = null }
  if (!el.value) return
  u = new uPlot(opts(), uData.value, el.value)
  applyFocus()
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
watch(() => props.focusNames, applyFocus, { deep: true })
</script>

<template>
  <div>
    <div ref="el" class="w-full"></div>
    <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs">
      <template v-if="showLegend">
        <button v-for="s in legend" :key="s.name" type="button"
          @mouseenter="emit('legend-hover', s.name)" @mouseleave="emit('legend-hover', null)"
          @click="emit('legend-toggle', s.name)"
          class="flex items-center gap-1.5 rounded transition-opacity"
          :class="[isDim(s.name) ? 'opacity-35' : '', isSel(s.name) ? 'font-medium text-fg' : '']">
          <span class="h-2 w-2 rounded-full" :class="isSel(s.name) ? 'ring-2 ring-offset-1 ring-offset-surface' : ''" :style="{ background: s.color, '--tw-ring-color': s.color }"></span>
          <span :class="isSel(s.name) ? 'text-fg' : 'text-muted'">{{ s.name }}</span>
          <span v-if="s.value" class="tabular-nums text-fg">{{ s.value }}</span>
        </button>
      </template>
      <span class="ml-auto tabular-nums text-faint">{{ hoverIdx != null ? cursorTime : 'now' }}</span>
    </div>
  </div>
</template>

<style>
.uplot { font-family: ui-monospace, monospace; }
</style>
