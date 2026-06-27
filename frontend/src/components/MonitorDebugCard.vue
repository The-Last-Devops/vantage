<script setup>
// Presentational "Last request / response" card for a service monitor: shows the
// last success and last failure payloads with a Copy button. Formatting + copy
// behaviour are passed in from the parent.
defineProps({
  debug: { type: Object, required: true },
  fmtDebug: { type: Function, required: true },
  copy: { type: Function, required: true },
})
</script>

<template>
  <div class="rounded-xl border border-line bg-surface p-4">
    <div class="mb-2 text-[11px] uppercase tracking-wider text-faint">Last request / response</div>
    <div class="grid gap-4 lg:grid-cols-2">
      <div>
        <div class="mb-1 flex items-center justify-between">
          <span class="text-xs font-medium text-accent">Last success</span>
          <button v-if="debug.ok" @click="copy(debug.ok, $event)" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
        </div>
        <pre v-if="debug.ok" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debug.ok) }}</pre>
        <p v-else class="text-xs text-faint">No successful check recorded yet.</p>
      </div>
      <div>
        <div class="mb-1 flex items-center justify-between">
          <span class="text-xs font-medium text-red-400">Last failure</span>
          <button v-if="debug.err" @click="copy(debug.err, $event)" class="rounded-md border border-line bg-surface2 px-2 py-0.5 text-xs text-muted hover:text-accent">Copy</button>
        </div>
        <pre v-if="debug.err" class="max-h-72 overflow-auto rounded-lg border border-line bg-bg p-3 text-xs leading-relaxed text-fg">{{ fmtDebug(debug.err) }}</pre>
        <p v-else class="text-xs text-faint">No failure recorded.</p>
      </div>
    </div>
  </div>
</template>
