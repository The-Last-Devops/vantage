<script setup>
// Presentational header card for a service monitor: status dot/label/duration
// + type / namespace / interval / latency / target. All values are passed in;
// the parent owns the monitor object and derived status.
defineProps({
  m: { type: Object, required: true },
  status: { type: String, required: true },
  statusLabel: { type: Object, required: true },
  statusColor: { type: Object, required: true },
  dotColor: { type: Object, required: true },
  dur: { type: Function, required: true },
  pushUrl: { type: String, default: '' },
})
</script>

<template>
  <div class="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border border-line bg-surface p-4">
    <div class="flex items-center gap-2">
      <span class="h-2.5 w-2.5 rounded-full" :class="dotColor[status]"></span>
      <span class="text-lg font-semibold" :class="statusColor[status]">{{ statusLabel[status] }}</span>
      <span v-if="status === 'up' || status === 'down'" class="text-sm text-muted">for {{ dur(m.since) }}</span>
    </div>
    <div class="text-sm text-muted"><span class="text-faint">Type</span> {{ m.kind }}</div>
    <div class="text-sm text-muted"><span class="text-faint">Namespace</span> {{ m.namespace }}</div>
    <div class="text-sm text-muted"><span class="text-faint">Interval</span> {{ m.interval_secs }}s</div>
    <div v-if="m.latency_ms != null" class="text-sm text-muted"><span class="text-faint">Latency</span> {{ m.latency_ms }} ms</div>
    <div class="min-w-0 flex-1 truncate text-right font-mono text-xs text-muted" v-tip="m.kind === 'push' ? pushUrl : m.target">
      {{ m.kind === 'push' ? pushUrl : m.target }}
    </div>
  </div>
</template>
