<script setup>
// Shared "Needs attention" incident list — used by Overview + FleetOverview.
// Each incident = { id, tone ('down'|'warn'), host, reason, ws, systemId? }.
// A down/warn row gets a tinted bg + a 3px left rail in its severity colour, the
// host name in mono, the reason, and (when a host id is known) an inline accent
// SSH/Exec button that routes to the host console. No incidents → calm "All clear".
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import VIcon from './VIcon.vue'
import StatePill from './StatePill.vue'

const props = defineProps({
  incidents: { type: Array, default: () => [] }, // [{ id, tone, host, reason, ws, systemId }]
  // Hide the card chrome (header) — the parent supplies its own.
  bare: { type: Boolean, default: false },
  // Cap the list height and scroll inside, so a long incident list can't take over
  // the whole page (the header + count stay pinned above the scroll area).
  scroll: { type: Boolean, default: false },
})
const router = useRouter()
const count = computed(() => props.incidents.length)
const RAIL = {
  down: 'bg-down/12 shadow-[inset_3px_0_0_rgb(var(--down))]',
  crit: 'bg-crit/12 shadow-[inset_3px_0_0_rgb(var(--crit))]',
  warn: 'bg-warn/12 shadow-[inset_3px_0_0_rgb(var(--warn))]',
  pending: 'bg-pending/12 shadow-[inset_3px_0_0_rgb(var(--pending))]',
}
const LABEL = { down: 'Down', crit: 'Critical', warn: 'Warn', ok: 'Up' }
function openConsole(inc) {
  if (inc.systemId) router.push({ name: 'console', params: { id: inc.systemId } })
}
</script>

<template>
  <div class="overflow-hidden rounded-xl border border-line bg-surface">
    <div v-if="!bare" class="flex items-center gap-2 border-b border-line bg-down/12 px-4 py-3">
      <StatePill tone="down" label="Needs attention" />
      <span class="ml-auto text-xs text-muted">{{ count }} active</span>
    </div>

    <!-- all clear -->
    <div v-if="!count" class="flex flex-col items-center justify-center gap-2 px-6 py-10 text-center">
      <span class="grid h-10 w-10 place-items-center rounded-full bg-ok/12 text-ok"><VIcon name="check-circle" :size="22" /></span>
      <div class="text-body font-medium text-fg">All clear</div>
      <p class="text-xs text-muted">No hosts down and no alerts firing.</p>
    </div>

    <ul v-else class="divide-y divide-line" :class="scroll ? 'max-h-[22rem] overflow-y-auto' : ''">
      <li v-for="inc in incidents" :key="inc.id"
        class="flex items-center gap-3 px-4 py-2.5" :class="RAIL[inc.tone] || RAIL.warn">
        <StatePill :tone="inc.tone" :label="LABEL[inc.tone] || 'Warn'" />
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-mono text-body tabular-nums text-fg">{{ inc.host }}</span>
            <span v-if="inc.ws" class="shrink-0 rounded bg-surface2 px-1.5 py-0.5 text-micro text-muted">{{ inc.ws }}</span>
          </div>
          <div class="truncate text-xs text-muted">{{ inc.reason }}</div>
        </div>
        <button v-if="inc.systemId" @click.stop="openConsole(inc)" v-tip="'Open console'"
          class="flex shrink-0 items-center gap-1 rounded-lg bg-accent px-2.5 py-1.5 font-mono text-micro font-semibold text-accentfg hover:opacity-90">
          <VIcon name="terminal" :size="12" /> SSH
        </button>
      </li>
    </ul>
  </div>
</template>
