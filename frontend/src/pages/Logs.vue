<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'

const loaded = ref(false)
const lines = ref([])
const err = ref('')
const q = ref('')
const auto = ref(true)
const copied = ref(false)
const box = ref(null)

const filtered = computed(() => {
  const n = q.value.trim().toLowerCase()
  return n ? lines.value.filter((l) => l.toLowerCase().includes(n)) : lines.value
})

async function load(first = false) {
  try {
    const r = await api.get('/api/admin/logs')
    const atBottom = !box.value || box.value.scrollTop + box.value.clientHeight >= box.value.scrollHeight - 40
    lines.value = r.lines || []
    err.value = ''
    if (atBottom) nextTick(() => { if (box.value) box.value.scrollTop = box.value.scrollHeight })
  } catch (e) {
    err.value = e.status === 403 ? 'Admin only.' : `Failed to load logs (${e.status || '?'}).`
  } finally {
    if (first) loaded.value = true
  }
}

let timer = null
function restart() { clearInterval(timer); if (auto.value) timer = setInterval(() => load(false), 5000) }
function toggleAuto() { auto.value = !auto.value; restart() }
onMounted(() => { load(true); restart() })
onBeforeUnmount(() => clearInterval(timer))

async function copy() {
  try { await navigator.clipboard.writeText(filtered.value.join('\n')); copied.value = true; setTimeout(() => (copied.value = false), 1500) } catch {}
}
function download() {
  const blob = new Blob([lines.value.join('\n')], { type: 'text/plain' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = 'vantage-hub-logs.txt'
  a.click()
  URL.revokeObjectURL(a.href)
}
// colour the level token so errors/warns stand out
const toneOf = (l) => (/\bERROR\b/.test(l) ? 'text-down' : /\bWARN\b/.test(l) ? 'text-warn' : /\bDEBUG\b|\bTRACE\b/.test(l) ? 'text-faint' : 'text-muted')
</script>

<template>
  <AppShell title="Logs">
    <PageLoader v-if="!loaded" />
    <div v-else class="space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <p class="mr-auto text-xs text-faint">Recent hub logs (in-memory, newest at bottom). For deeper history use <code class="rounded bg-surface2 px-1 py-0.5">kubectl logs</code>.</p>
        <input v-model="q" placeholder="Filter…" class="w-48 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
        <button @click="toggleAuto" class="rounded-lg border border-line px-3 py-1.5 text-sm" :class="auto ? 'bg-accent/12 text-accent' : 'text-muted hover:text-fg'">{{ auto ? 'Auto ⟳' : 'Paused' }}</button>
        <button @click="load(false)" class="rounded-lg border border-line px-3 py-1.5 text-sm text-muted hover:text-fg">Refresh</button>
        <button @click="copy" class="rounded-lg border border-line px-3 py-1.5 text-sm text-muted hover:text-fg">{{ copied ? 'Copied ✓' : 'Copy' }}</button>
        <button @click="download" class="rounded-lg border border-line px-3 py-1.5 text-sm text-muted hover:text-fg">Download</button>
      </div>
      <p v-if="err" class="rounded-lg border border-down/40 bg-down/10 px-3 py-2 text-sm text-down">{{ err }}</p>
      <div ref="box" class="h-[70vh] overflow-auto rounded-2xl border border-line bg-surface p-3 font-mono text-xs leading-relaxed">
        <p v-if="!filtered.length" class="text-faint">No log lines{{ q ? ' match the filter' : ' yet' }}.</p>
        <div v-for="(l, i) in filtered" :key="i" class="whitespace-pre-wrap break-all" :class="toneOf(l)">{{ l }}</div>
      </div>
      <p class="text-xs text-faint">{{ filtered.length }}<span v-if="q"> / {{ lines.length }}</span> lines</p>
    </div>
  </AppShell>
</template>
