<script setup>
// Fleet metrics — New-Relic-style small multiples. One panel per metric, every
// host overlaid as a <polyline>; click a host (rail row or banner) to isolate its
// line across all panels. Reads the EXISTING /api/fleet + /api/systems endpoints
// only — no backend changes. Charts are inline SVG (self-contained), not uPlot.
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { useCached } from '../lib/cache'

const route = useRoute()

// ---- namespace filter (?ns=a,b ; empty = all) — same contract as Systems.vue ----
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const inNs = (ns) => selectedNs.value.length === 0 || selectedNs.value.includes(ns)

// ---- range control (segmented; default 24h) — refetches /api/fleet?range= ----
const RANGES = ['1h', '6h', '24h', '7d']
const range = computed(() => (RANGES.includes(route.query.mrange) ? route.query.mrange : '24h'))
const rtr = useRouter()
function setRange(r) { rtr.replace({ query: { ...route.query, mrange: r } }) }

// ---- data: /api/systems (name → id + namespace) and /api/fleet (series) -------
const systems = ref([])
const fleet = ref(null)

// host name → { id, namespace } map, from /api/systems (same shape Systems.vue uses)
const hostInfo = computed(() => {
  const m = {}
  for (const s of systems.value) m[s.name] = { id: s.id, namespace: s.namespace }
  return m
})

// stable host → color map (sorted by name) so a host's line, swatch and rail row
// all match — read no theme token here on purpose: distinct hues need a generated
// palette, like Systems.vue's colorOf (hsl strings, not raw hex, so the token
// guard is satisfied).
const colorOf = computed(() => {
  const names = [...new Set(systems.value.map((s) => s.name))].sort()
  const m = {}
  names.forEach((n, i) => { m[n] = `hsl(${(i * 47) % 360} 70% 58%)` })
  return m
})

// names visible under the current namespace selection
const visibleNames = computed(() => {
  const out = new Set()
  for (const s of systems.value) if (inNs(s.namespace)) out.add(s.name)
  return out
})

// ---- selection / isolate-on-click --------------------------------------------
// A single selected host name isolates its line everywhere; click again to clear.
const selected = ref(null)
function toggle(name) { selected.value = selected.value === name ? null : name }
function clearSel() { selected.value = null }

// ---- metric panel definitions ------------------------------------------------
// cpu/mem/disk scale 0..100; network scales to its own max in the visible data.
const PANELS = [
  { key: 'cpu', title: 'CPU', unit: '%', fixedMax: 100 },
  { key: 'mem', title: 'Memory', unit: '%', fixedMax: 100 },
  { key: 'disk', title: 'Disk', unit: '%', fixedMax: 100 },
  { key: 'net', title: 'Network', unit: 'B/s', fixedMax: null },
]

const lastNonNull = (d) => { if (!d) return null; for (let i = d.length - 1; i >= 0; i--) if (d[i] != null) return d[i]; return null }

// series for one metric, filtered to the namespace selection
function seriesFor(key) {
  const arr = (fleet.value && fleet.value[key]) || []
  return arr
    .filter((s) => visibleNames.value.has(s.name))
    .map((s) => ({ name: s.name, color: colorOf.value[s.name] || 'hsl(0 0% 55%)', data: s.data }))
}

// max for scaling: fixed (100) for %, else the max non-null across visible series
function maxFor(panel, series) {
  if (panel.fixedMax) return panel.fixedMax
  let m = 0
  for (const s of series) for (const v of s.data) if (v != null && v > m) m = v
  return m || 1
}

// build an SVG polyline `points` string for a series. viewBox is 0..100 (x) by
// 0..40 (y, inverted so larger = higher). Nulls break the line into segments.
function polylines(data, n, max) {
  const W = 100, H = 40
  const segs = []
  let cur = []
  data.forEach((v, i) => {
    if (v == null) { if (cur.length) { segs.push(cur); cur = [] } return }
    const x = n > 1 ? (i / (n - 1)) * W : 0
    const y = H - Math.min(1, v / max) * H
    cur.push(`${x.toFixed(2)},${y.toFixed(2)}`)
  })
  if (cur.length) segs.push(cur)
  return segs.map((s) => s.join(' '))
}

// per-panel render model: title, current value (selected host, else fleet avg),
// and the list of host polylines
const panels = computed(() => {
  const t = (fleet.value && fleet.value.t) || []
  const n = t.length
  return PANELS.map((p) => {
    const series = seriesFor(p.key)
    const max = maxFor(p, series)
    const lines = series.map((s) => ({
      name: s.name,
      color: s.color,
      segs: polylines(s.data, n, max),
    }))
    // header value: selected host's latest, else fleet average of latest values
    let value = null
    if (selected.value) {
      const s = series.find((x) => x.name === selected.value)
      value = s ? lastNonNull(s.data) : null
    } else {
      const vals = series.map((s) => lastNonNull(s.data)).filter((v) => v != null)
      value = vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null
    }
    return { ...p, lines, value }
  })
})

// format a metric value for a panel header / rail
function fmt(unit, v) {
  if (v == null) return '—'
  if (unit === '%') return Math.round(v) + '%'
  // bytes/sec
  const us = ['B', 'K', 'M', 'G']; let i = 0; let nn = v
  while (nn >= 1024 && i < 3) { nn /= 1024; i++ }
  return (nn < 10 && i > 0 ? nn.toFixed(1) : Math.round(nn)) + ' ' + us[i] + '/s'
}

// ---- host rail: one row per visible host, sorted by name (stable) ------------
// latest CPU% per host name, for the rail value
const latestCpu = computed(() => {
  const m = {}
  for (const s of seriesFor('cpu')) m[s.name] = lastNonNull(s.data)
  return m
})
const railHosts = computed(() =>
  [...visibleNames.value]
    .sort((a, b) => a.localeCompare(b))
    .map((name) => ({ name, color: colorOf.value[name] || 'hsl(0 0% 55%)', cpu: latestCpu.value[name] ?? null, id: hostInfo.value[name]?.id }))
)

const hostCount = computed(() => visibleNames.value.size)
const selectedId = computed(() => (selected.value ? hostInfo.value[selected.value]?.id : null))

// ---- loaders -----------------------------------------------------------------
// /api/systems is global; /api/fleet is keyed by range. Cache key folds in the
// range + namespace selection so a switch repaints from cache then revalidates.
const { loaded, reload } = useCached({
  key: () => 'metrics:' + range.value + ':' + selectedNs.value.join(','),
  load: async () => {
    const [sys, fl] = await Promise.all([
      api.get('/api/systems'),
      api.get(`/api/fleet?range=${range.value}`),
    ])
    return { sys, fl }
  },
  apply: ({ sys, fl }) => { systems.value = sys; fleet.value = fl },
})

let timer = null
onMounted(() => { reload(); timer = setInterval(reload, 5000) })
onUnmounted(() => clearInterval(timer))
// refetch when the range or namespace selection changes
watch([range, selectedNs], reload)
</script>

<template>
  <AppShell title="Metrics">
    <div class="space-y-4">
      <!-- top bar: crumb + range segmented control + live dot -->
      <div class="flex flex-wrap items-center gap-3">
        <div class="flex items-center gap-2">
          <h2 class="text-sm font-semibold text-fg">Metrics</h2>
          <span class="rounded-full bg-surface2 px-2 py-0.5 text-xs text-muted">{{ hostCount }} host{{ hostCount === 1 ? '' : 's' }}</span>
        </div>
        <div class="ml-auto flex items-center gap-3">
          <span class="flex items-center gap-1.5 text-xs text-muted">
            <span class="relative flex h-2 w-2">
              <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-ok opacity-60"></span>
              <span class="relative inline-flex h-2 w-2 rounded-full bg-ok"></span>
            </span>
            live
          </span>
          <div class="flex rounded-lg border border-line bg-surface2 p-0.5 text-xs">
            <button v-for="r in RANGES" :key="r" @click="setRange(r)" class="rounded-md px-2.5 py-1"
              :class="range === r ? 'bg-accent/15 font-medium text-accent' : 'text-muted hover:text-fg'">{{ r }}</button>
          </div>
        </div>
      </div>

      <!-- selection banner: viewing <host> + open host link + clear -->
      <div v-if="selected" class="flex flex-wrap items-center gap-3 rounded-[11px] border border-accent/40 bg-accent/10 px-3 py-2 text-sm">
        <span class="flex items-center gap-2">
          <span class="h-2.5 w-2.5 rounded-full" :style="{ background: colorOf[selected] }"></span>
          <span class="text-fg">Viewing <span class="font-mono">{{ selected }}</span></span>
        </span>
        <div class="ml-auto flex items-center gap-3">
          <RouterLink v-if="selectedId" :to="{ name: 'system', params: { id: selectedId } }" class="flex items-center gap-1 text-accent hover:underline">
            Open host
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M7 17 17 7M7 7h10v10"/></svg>
          </RouterLink>
          <button @click="clearSel" class="text-xs text-muted hover:text-fg">Clear</button>
        </div>
      </div>

      <PageLoader v-if="!loaded" />

      <template v-else>
        <p v-if="!hostCount" class="rounded-[11px] border border-line bg-surface p-6 text-center text-sm text-muted">
          No hosts in the selected namespaces.
        </p>

        <div v-else class="grid grid-cols-1 gap-4 lg:grid-cols-[1fr_240px]">
          <!-- metric panels -->
          <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <div v-for="p in panels" :key="p.key" class="rounded-[11px] border border-line bg-surface p-3">
              <div class="mb-2 flex items-baseline justify-between gap-2">
                <div class="text-sm font-medium text-fg">{{ p.title }} <span class="text-xs text-faint">{{ p.unit }}</span></div>
                <div class="text-sm tabular-nums text-fg">
                  {{ fmt(p.unit, p.value) }}
                  <span class="ml-1 text-[10px] uppercase tracking-wide text-faint">{{ selected ? 'host' : 'avg' }}</span>
                </div>
              </div>
              <svg viewBox="0 0 100 40" preserveAspectRatio="none" class="h-28 w-full" role="img" :aria-label="p.title">
                <template v-for="ln in p.lines" :key="ln.name">
                  <polyline v-for="(pts, si) in ln.segs" :key="ln.name + ':' + si"
                    :points="pts" fill="none"
                    :stroke="ln.color" vector-effect="non-scaling-stroke"
                    :stroke-width="selected === ln.name ? 2 : 1"
                    :stroke-opacity="!selected ? 0.85 : (selected === ln.name ? 1 : 0.12)" />
                </template>
              </svg>
            </div>
          </div>

          <!-- host rail -->
          <aside class="rounded-[11px] border border-line bg-surface p-2">
            <div class="px-1.5 pb-1.5 pt-1 text-[10px] uppercase tracking-wider text-muted">Hosts</div>
            <ul class="max-h-[70vh] space-y-0.5 overflow-y-auto">
              <li v-for="h in railHosts" :key="h.name">
                <button @click="toggle(h.name)"
                  class="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-xs transition-opacity"
                  :class="[
                    selected === h.name ? 'bg-accent/15' : 'hover:bg-hover',
                    selected && selected !== h.name ? 'opacity-45' : '',
                  ]">
                  <span class="h-2.5 w-2.5 shrink-0 rounded-full" :style="{ background: h.color }"></span>
                  <span class="flex-1 truncate font-mono text-fg" v-tip="h.name">{{ h.name }}</span>
                  <span class="tabular-nums text-muted">{{ h.cpu == null ? '—' : Math.round(h.cpu) + '%' }}</span>
                </button>
              </li>
            </ul>
          </aside>
        </div>
      </template>
    </div>
  </AppShell>
</template>
