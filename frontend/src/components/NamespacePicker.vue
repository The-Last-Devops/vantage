<script setup>
// Namespace switcher — moved out of the sidebar into the header right cluster.
// Multi-select that writes to the URL (?ns=a,b); empty = all namespaces. The
// selection is shareable and read back by every namespaced page. Behaviour is
// kept identical to the old sidebar block (toggleNs / toggleAll / label rules).
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../lib/api'

const route = useRoute()
const router = useRouter()

const open = ref(false)
const rootRef = ref(null)
const namespaces = ref([]) // [{id,name}]

const nsNames = computed(() => namespaces.value.map((n) => n.name))
// Selected namespaces live in the URL (?ns=a,b) so they're shareable; empty = all.
const selectedNs = computed(() => (route.query.ns || '').split(',').filter(Boolean))
const isAll = computed(() => selectedNs.value.length === 0 || selectedNs.value.length === nsNames.value.length)
const nsLabel = computed(() => {
  const n = selectedNs.value.length
  if (n === 0 || n === nsNames.value.length) return 'All namespaces'
  return n === 1 ? selectedNs.value[0] : `${n} namespaces`
})
const nsChecked = (name) => selectedNs.value.length === 0 || selectedNs.value.includes(name)
const allChecked = isAll

function setNs(arr) {
  const all = arr.length === 0 || arr.length === nsNames.value.length
  router.replace({ query: { ...route.query, ns: all ? undefined : arr.join(',') } })
}
function toggleNs(name) {
  // from "all" (nothing selected), a click picks just that namespace; further
  // clicks add/remove from the explicit selection
  if (selectedNs.value.length === 0) { setNs([name]); return }
  const cur = [...selectedNs.value]
  const i = cur.indexOf(name)
  if (i >= 0) cur.splice(i, 1); else cur.push(name)
  setNs(cur)
}
function toggleAllNs() { setNs([]) } // clears the filter → show all namespaces

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
})
// close the dropdown on any click outside it
function onDocClick(e) { if (open.value && rootRef.value && !rootRef.value.contains(e.target)) open.value = false }
onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))
</script>

<template>
  <div ref="rootRef" class="relative">
    <button @click="open = !open" v-tip="`Namespace filter`"
      class="flex items-center gap-1.5 rounded-lg border border-line2 bg-surface2 px-2.5 py-1.5 text-sm text-fg hover:border-accent/50">
      <VIcon name="globe" :size="14" class="shrink-0 text-muted" />
      <span class="max-w-[140px] truncate">{{ nsLabel }}</span>
      <VIcon name="chevron" :size="14" class="shrink-0 text-muted transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>
    <div v-if="open" class="absolute right-0 top-full z-30 mt-1 max-h-72 w-56 overflow-y-auto rounded-lg border border-line2 bg-surface2 py-1 shadow-xl">
      <button @click="toggleAllNs()" class="flex w-full items-center gap-2.5 border-b border-line px-3 py-2 text-left text-sm hover:bg-surface" :class="allChecked ? 'text-accent' : 'text-muted'">
        <span class="grid h-4 w-4 place-items-center rounded border" :class="allChecked ? 'border-accent bg-accent' : 'border-line'"></span>All namespaces
      </button>
      <button v-for="n in namespaces" :key="n.id" @click="toggleNs(n.name)"
        class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface" :class="nsChecked(n.name) ? 'text-fg' : 'text-muted'">
        <span class="grid h-4 w-4 place-items-center rounded border" :class="nsChecked(n.name) ? 'border-accent bg-accent' : 'border-line'"></span>{{ n.name }}
      </button>
    </div>
  </div>
</template>
