<script setup>
// Workspace switcher — moved out of the sidebar into the header right cluster.
// Multi-select that writes to the URL (?ws=a,b); empty = all workspaces. The
// selection is shareable and read back by every workspaced page. Behaviour is
// kept identical to the old sidebar block (toggleWs / toggleAll / label rules).
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'

const route = useRoute()
const router = useRouter()

const open = ref(false)
const rootRef = ref(null)
const workspaces = ref([]) // [{id,name}]

const wsNames = computed(() => workspaces.value.map((n) => n.name))
// Selected workspaces live in the URL (?ws=a,b) so they're shareable; empty = all.
const selectedWs = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const isAll = computed(() => selectedWs.value.length === 0 || selectedWs.value.length === wsNames.value.length)
const wsLabel = computed(() => {
  const n = selectedWs.value.length
  if (n === 0 || n === wsNames.value.length) return 'All workspaces'
  return n === 1 ? selectedWs.value[0] : `${n} workspaces`
})
const wsChecked = (name) => selectedWs.value.length === 0 || selectedWs.value.includes(name)
const allChecked = isAll

function setWs(arr) {
  const all = arr.length === 0 || arr.length === wsNames.value.length
  router.replace({ query: { ...route.query, ws: all ? undefined : arr.join(',') } })
}
function toggleWs(name) {
  // from "all" (nothing selected), a click picks just that workspace; further
  // clicks add/remove from the explicit selection
  if (selectedWs.value.length === 0) { setWs([name]); return }
  const cur = [...selectedWs.value]
  const i = cur.indexOf(name)
  if (i >= 0) cur.splice(i, 1); else cur.push(name)
  setWs(cur)
}
function toggleAllWs() { setWs([]) } // clears the filter → show all workspaces

onMounted(async () => {
  try { workspaces.value = await api.get('/api/workspaces') } catch { workspaces.value = [] }
})
// close the dropdown on any click outside it
function onDocClick(e) { if (open.value && rootRef.value && !rootRef.value.contains(e.target)) open.value = false }
onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))
</script>

<template>
  <div ref="rootRef" class="relative">
    <button @click="open = !open" v-tip="`Workspace filter`"
      class="flex items-center gap-1.5 rounded-lg border border-line2 bg-surface2 px-2.5 py-1.5 text-sm text-fg hover:border-accent/50">
      <VIcon name="globe" :size="14" class="shrink-0 text-muted" />
      <span class="max-w-[140px] truncate">{{ wsLabel }}</span>
      <VIcon name="chevron" :size="14" class="shrink-0 text-muted transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>
    <div v-if="open" class="absolute right-0 top-full z-30 mt-1 max-h-72 w-56 overflow-y-auto rounded-lg border border-line2 bg-surface2 py-1 shadow-xl">
      <button @click="toggleAllWs()" class="flex w-full items-center gap-2.5 border-b border-line px-3 py-2 text-left text-sm hover:bg-surface" :class="allChecked ? 'text-accent' : 'text-muted'">
        <span class="grid h-4 w-4 place-items-center rounded border" :class="allChecked ? 'border-accent bg-accent' : 'border-line'"></span>All workspaces
      </button>
      <button v-for="n in workspaces" :key="n.id" @click="toggleWs(n.name)"
        class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface" :class="wsChecked(n.name) ? 'text-fg' : 'text-muted'">
        <span class="grid h-4 w-4 place-items-center rounded border" :class="wsChecked(n.name) ? 'border-accent bg-accent' : 'border-line'"></span>{{ n.name }}
      </button>
    </div>
  </div>
</template>
