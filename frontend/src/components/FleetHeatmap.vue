<script setup>
// Fleet health heatmap — hosts grouped by workspace, each a 20px rounded tile
// coloured by state. Hover → host name (v-tip), click → its detail page.
// `groups` = [{ name, hosts: [{ id, name, state }] }], state ∈ ok|warn|down|pending|disabled.
import { useRouter } from 'vue-router'
import VIcon from './VIcon.vue'

defineProps({
  groups: { type: Array, default: () => [] },
})
const router = useRouter()
const TILE = {
  ok: 'bg-ok/70',
  warn: 'bg-warn',
  crit: 'bg-crit',
  down: 'bg-down ring-2 ring-down/45',
  pending: 'bg-pending',
  disabled: 'bg-surface2 border border-line',
}
const LEGEND = [
  ['ok', 'OK'],
  ['warn', 'Warning'],
  ['crit', 'Critical'],
  ['down', 'Down'],
  ['pending', 'Pending'],
  ['disabled', 'Disabled'],
]
function open(h) {
  router.push({ name: 'system', params: { id: h.id } })
}
</script>

<template>
  <div class="rounded-xl border border-line bg-surface p-4">
    <div class="mb-3 flex items-center gap-2">
      <VIcon name="fleet" :size="16" class="text-muted" />
      <h2 class="text-h2 font-semibold text-fg">Fleet health</h2>
    </div>

    <div v-if="!groups.length" class="py-6 text-center text-xs text-muted">No hosts.</div>

    <div v-else class="space-y-3">
      <div v-for="g in groups" :key="g.name">
        <div class="mb-1.5 flex items-center gap-2 text-xs text-muted">
          <span class="font-medium">{{ g.name }}</span>
          <span class="text-faint">{{ g.hosts.length }}</span>
        </div>
        <div class="flex flex-wrap gap-1.5">
          <button v-for="h in g.hosts" :key="h.id" v-tip="`${h.name} · ${h.state}`" @click="open(h)"
            class="h-5 w-5 rounded-[5px] transition-transform hover:scale-110" :class="TILE[h.state] || TILE.disabled"></button>
        </div>
      </div>
    </div>

    <!-- legend -->
    <div class="mt-4 flex flex-wrap items-center gap-x-3 gap-y-1.5 border-t border-line pt-3 text-micro text-muted">
      <span v-for="[s, label] in LEGEND" :key="s" class="flex items-center gap-1.5">
        <span class="h-3 w-3 rounded-[4px]" :class="TILE[s]"></span>{{ label }}
      </span>
    </div>
  </div>
</template>
