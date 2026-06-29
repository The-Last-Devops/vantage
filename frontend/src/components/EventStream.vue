<script setup>
// Presentational status-change feed for service monitors. Rows are passed in
// pre-shaped by the parent (it owns fetching + namespace filtering); formatting
// helpers come in as props so this stays purely presentational. The list is
// height-capped (scrolls inside) and paginated so a chatty feed can't grow the
// page without bound. `stateDur` is indexed against the FULL list, so we pass the
// global index (page offset + row) when rendering a page.
import { ref, computed, watch } from 'vue'

const props = defineProps({
  events: { type: Array, default: () => [] },
  evTime: { type: Function, required: true },
  evMessage: { type: Function, required: true },
  // (i) => { secs, ongoing } — duration the event's state lasted
  stateDur: { type: Function, required: true },
  fmtDur: { type: Function, required: true },
  // whether to show the per-event service name + link (list view), off on detail
  showService: { type: Boolean, default: true },
})

const PAGE = 25
const page = ref(1)
const pages = computed(() => Math.max(1, Math.ceil(props.events.length / PAGE)))
const start = computed(() => (page.value - 1) * PAGE)
const shown = computed(() => props.events.slice(start.value, start.value + PAGE))
watch(() => props.events, () => { if (page.value > pages.value) page.value = 1 })
</script>

<template>
  <div class="overflow-hidden rounded-xl border border-line bg-surface">
    <div class="flex items-center gap-2 border-b border-line2 bg-head px-4 py-2.5">
      <VIcon name="pulse" :size="16" class="text-faint" />
      <h2 class="text-xs font-extrabold uppercase tracking-wide text-fg">Recent events</h2>
      <span class="rounded-pill bg-surface2 px-2 py-0.5 text-micro text-muted">{{ events.length }}</span>
    </div>
    <p v-if="!events.length" class="px-4 py-8 text-center text-sm text-muted">No status changes recorded recently.</p>
    <template v-else>
      <ul class="max-h-[min(430px,calc(100vh-15rem))] divide-y divide-line overflow-y-auto">
        <li v-for="(e, i) in shown" :key="start + i" class="flex items-start gap-3 px-4 py-2.5 hover:bg-hover">
          <StatePill :tone="e.up ? 'ok' : 'down'" :label="e.up ? 'Up' : 'Down'" class="mt-0.5 shrink-0" />
          <div class="min-w-0 flex-1">
            <RouterLink v-if="showService" :to="{ name: 'monitor', params: { id: e.monitor_id } }"
              class="block font-mono text-sm text-fg hover:text-accent" @click.stop>{{ e.name }}</RouterLink>
            <div class="text-sm text-muted break-words">{{ evMessage(e) }}</div>
            <div class="mt-0.5 flex items-center gap-1.5 text-micro font-mono tabular-nums text-faint">
              <span>{{ evTime(e.at) }}</span>
              <span>·</span>
              <span>{{ fmtDur(stateDur(start + i).secs) }}<span v-if="stateDur(start + i).ongoing"> · ongoing</span></span>
            </div>
          </div>
        </li>
      </ul>
      <div v-if="pages > 1" class="flex items-center justify-between border-t border-line px-4 py-2.5 text-xs">
        <button :disabled="page <= 1" @click="page--"
          class="rounded-lg border border-line px-2.5 py-1 text-muted hover:border-accent/50 hover:text-fg disabled:opacity-40">Prev</button>
        <span class="font-mono tabular-nums text-faint">Page {{ page }} / {{ pages }}</span>
        <button :disabled="page >= pages" @click="page++"
          class="rounded-lg border border-line px-2.5 py-1 text-muted hover:border-accent/50 hover:text-fg disabled:opacity-40">Next</button>
      </div>
    </template>
  </div>
</template>
