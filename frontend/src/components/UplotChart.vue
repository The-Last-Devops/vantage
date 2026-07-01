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
  tooltip: { type: Boolean, default: false }, // floating cursor tooltip (overlay charts)
  legendValuesAlways: { type: Boolean, default: true }, // false → show values only on hover
  area: { type: Boolean, default: true }, // false → lines only (cleaner for many-host overlay)
  spanGaps: { type: Boolean, default: false }, // true → connect across missing buckets
  // legend interaction (controlled by parent): focusNames = series to show (others
  // hidden); null = show all. selectedNames = pinned series (for legend styling).
  focusNames: { type: Array, default: null },
  selectedNames: { type: Array, default: () => [] },
  viewRange: { type: Array, default: null }, // [minTs, maxTs] zoom window (persisted by parent); null = full
})
const emit = defineEmits(['legend-hover', 'legend-toggle', 'cursor-time', 'zoom'])
const isSel = (n) => props.selectedNames.includes(n)
const isDim = (n) => props.focusNames != null && !props.focusNames.includes(n)
const isHi = (n) => lineHover.value === n || (props.focusNames != null && props.focusNames.includes(n)) // bright
const short = (n) => (n && n.length > 10 ? n.slice(0, 10) + '…' : n)

const ui = useUi()
const el = ref(null)
const hoverIdx = ref(null) // data index under cursor; null when not hovering
const lineHover = ref(null) // series name nearest the cursor (local; brightens its legend row)
const tip = ref({ show: false, x: 0, y: 0, name: '', color: '', val: '', time: '' }) // floating tooltip
let u = null
let ro = null
let zoomed = false // user drag-zoomed → freeze the view; live data keeps appending off-screen
let dragging = false // pointer down on the plot → pause live redraws so the drag-select isn't wiped
let focusIdx = null // uPlot series index nearest the cursor (1-based; 0 is the x axis)

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
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
})

function opts() {
  const axis = cssVar('--muted'), grid = cssVar('--line')
  return {
    width: el.value?.clientWidth || 400,
    height: props.height,
    padding: [10, 8, 0, 0],
    legend: { show: false }, // we render our own
    focus: { alpha: 0.25 }, // hovering a line dims the others (canvas-level, cheap)
    cursor: {
      points: { size: 7 },
      focus: { prox: 30 },
      drag: { x: true, y: false, setScale: false }, // we zoom manually in setSelect (more reliable here)
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
      { stroke: axis, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid }, font: '11px ui-monospace, monospace', size: /B/.test(props.unit) ? 72 : 52, values: (_u, vals) => vals.map(fmt) },
    ],
    hooks: {
      setCursor: [(up) => {
        hoverIdx.value = up.cursor.idx
        if (!props.tooltip) return
        const { idx, left, top } = up.cursor
        const s = focusIdx ? props.series[focusIdx - 1] : null
        if (idx == null || !s) { tip.value = { ...tip.value, show: false }; return }
        const realIdx = Math.max(0, idx - padded.value.prepend)
        const ts = props.time[realIdx]
        tip.value = {
          show: true,
          x: up.over.offsetLeft + left,
          y: up.over.offsetTop + top,
          name: s.name,
          color: s.color,
          val: fmt(s.data[realIdx]),
          time: ts ? new Date(ts * 1000).toLocaleString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }) : '',
        }
      }],
      // nearest line under the cursor → brighten its legend row (local only; uPlot's
      // focus.alpha dims the other lines on the canvas). Click on the plot pins it.
      setSeries: [(_up, i, o) => {
        if (!o || o.focus == null) return
        if (o.focus) { focusIdx = i; lineHover.value = props.series[i - 1]?.name ?? null }
        else if (focusIdx === i) { focusIdx = null; lineHover.value = null }
      }],
      setScale: [(up, key) => {
        if (key !== 'x') return
        const t = up.data[0]
        if (!t || !t.length) { zoomed = false; return }
        // zoomed if the visible x-range is narrower than the full data extent
        zoomed = up.scales.x.min > t[0] + 1 || up.scales.x.max < t[t.length - 1] - 1
      }],
      // drag-release → zoom to the selected pixel range and persist it (parent → URL)
      setSelect: [(up) => {
        const w = up.select.width
        if (w > 4) {
          const min = up.posToVal(up.select.left, 'x')
          const max = up.posToVal(up.select.left + w, 'x')
          up.setScale('x', { min, max })
          emit('zoom', [min, max])
        }
        up.setSelect({ left: 0, width: 0, top: 0, height: 0 }, false) // clear the box
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

// apply the persisted zoom window (from the URL); null → reset to the full range
function applyViewRange() {
  if (!u) return
  const v = props.viewRange
  if (v && v.length === 2) u.setScale('x', { min: v[0], max: v[1] })
  else u.setData(uData.value, true)
}

function onUp() { dragging = false } // resume live redraws after the drag ends

function build() {
  if (u) { u.destroy(); u = null }
  if (!el.value) return
  u = new uPlot(opts(), uData.value, el.value)
  applyFocus()
  applyViewRange()
  u.over.addEventListener('mousedown', () => { dragging = true })
  u.over.addEventListener('mouseleave', () => { focusIdx = null; lineHover.value = null; tip.value = { ...tip.value, show: false } })
  // a plain click (no drag) on/near a line pins that host; real drags fire no click
  u.over.addEventListener('click', () => { if (focusIdx) emit('legend-toggle', props.series[focusIdx - 1]?.name) })
  // double-click resets the zoom back to the full window
  u.over.addEventListener('dblclick', () => { emit('zoom', null); u.setData(uData.value, true) })
}

onMounted(() => {
  build()
  ro = new ResizeObserver(() => u && u.setSize({ width: el.value.clientWidth, height: props.height }))
  ro.observe(el.value)
  window.addEventListener('mouseup', onUp) // release may land outside the plot
})
onBeforeUnmount(() => { ro && ro.disconnect(); window.removeEventListener('mouseup', onUp); u && u.destroy() })
// follow the latest when not zoomed; when a zoom window is set, re-pin it on every
// update (survives live polling + F5); never redraw mid-drag (wipes the selection)
watch(uData, (d) => {
  if (!u || dragging) return
  const v = props.viewRange
  if (v && v.length === 2) { u.setData(d, false); u.setScale('x', { min: v[0], max: v[1] }) }
  else u.setData(d, !zoomed)
})
watch(() => ui.light, () => build())
// uPlot fixes its series at build time, so rebuild when the set of series changes
// (e.g. container/node lines arriving after the host, or the filter changing them)
watch(() => props.series.map((s) => s.name).join(''), () => build())
watch(() => props.focusNames, applyFocus, { deep: true })
watch(() => props.viewRange, applyViewRange, { deep: true })
// surface the hovered timestamp ('now' when not hovering) so the parent can show
// it in the chart header — keeps it out of the legend so nothing reflows
watch([hoverIdx, cursorTime], () => emit('cursor-time', hoverIdx.value != null ? cursorTime.value : ''), { immediate: true })
</script>

<template>
  <div>
    <div class="relative">
      <div ref="el" class="w-full"></div>
      <!-- floating cursor tooltip (overlay charts): nearest host + value at time -->
      <div v-if="tooltip && tip.show" class="pointer-events-none absolute z-20 -translate-x-1/2 -translate-y-[calc(100%+10px)] whitespace-nowrap rounded-md border border-line bg-surface2 px-2 py-1 text-xs shadow-xl" :style="{ left: tip.x + 'px', top: tip.y + 'px' }">
        <div class="text-faint">{{ tip.time }}</div>
        <div class="mt-0.5 flex items-center gap-1.5"><span class="h-2 w-2 rounded-full" :style="{ background: tip.color }"></span><span class="text-fg">{{ tip.name }}</span><span class="font-mono tabular-nums font-medium text-fg">{{ tip.val }}</span></div>
      </div>
    </div>
    <!-- fixed-column grid so values appearing on hover never change the row
         count (→ no height jump); the time sits on its own always-present line -->
    <!-- flex-wrap so few series sit on one row; the value slot has a reserved
         min-width so values appearing on hover don't reflow the row -->
    <div v-if="showLegend" @mouseleave="emit('legend-hover', null)" class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs">
      <button v-for="s in legend" :key="s.name" type="button" v-tip="s.name"
        @mouseenter="emit('legend-hover', s.name)"
        @click="emit('legend-toggle', s.name)"
        class="flex items-center gap-1.5 rounded transition-opacity"
        :class="isDim(s.name) ? 'opacity-35' : ''">
        <span class="h-2 w-2 shrink-0 rounded-full" :class="isSel(s.name) ? 'ring-2 ring-offset-1 ring-offset-surface' : ''" :style="{ background: s.color, '--tw-ring-color': s.color }"></span>
        <span :class="isSel(s.name) || isHi(s.name) ? 'text-fg' : 'text-muted'">{{ short(s.name) }}</span>
        <span class="min-w-[2.5em] text-right font-mono tabular-nums text-fg">{{ s.value }}</span>
      </button>
    </div>
  </div>
</template>

<style>
.uplot { font-family: ui-monospace, monospace; }
/* visible drag-to-select overlay (uPlot's default is nearly invisible on dark) */
.uplot .u-select { background: rgb(var(--accent) / 0.18); border-left: 1px solid rgb(var(--accent) / 0.6); border-right: 1px solid rgb(var(--accent) / 0.6); }
</style>
