<script setup>
// Utilisation gauge coloured by the SAME thresholds that decide a host's Warn/Critical
// state (per-workspace, falling back to DEFAULT_THR). It used to hard-code 70/90 while
// triage used 80/90, so a 73% disk read amber on a host the Needs-attention view called
// fine. Callers pass the workspace's `<metric>_warn` / `<metric>_crit`.
import { DEFAULT_THR } from '../lib/triage'
const props = defineProps({
  v: { default: null },
  warn: { type: Number, default: DEFAULT_THR.cpu_warn },
  crit: { type: Number, default: DEFAULT_THR.cpu_crit },
})
const cls = (x) => (x >= props.crit ? 'bg-down' : x >= props.warn ? 'bg-warn' : 'bg-accent')
const tcls = (x) => (x >= props.crit ? 'text-down' : x >= props.warn ? 'text-warn' : 'text-fg')
</script>

<template>
  <span v-if="v == null" class="text-faint">—</span>
  <div v-else class="flex items-center gap-2">
    <div class="h-1.5 w-16 overflow-hidden rounded bg-line"><div class="h-full" :class="cls(v)" :style="{ width: v + '%' }"></div></div>
    <span class="font-mono tabular-nums" :class="tcls(v)">{{ v }}%</span>
  </div>
</template>
