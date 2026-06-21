<script setup>
import { ref, computed } from 'vue'

// v-model = the raw query string; items = systems (to suggest real ns/agent values).
const props = defineProps({ modelValue: { type: String, default: '' }, items: { type: Array, default: () => [] } })
const emit = defineEmits(['update:modelValue'])

const text = computed({ get: () => props.modelValue, set: (v) => emit('update:modelValue', v) })
const open = ref(false)
const hi = ref(0)

const FIELDS = [
  { insert: 'name:', label: 'name:', desc: 'node name (default)' },
  { insert: 'cpu>', label: 'cpu', desc: 'cpu %  (cpu>50)' },
  { insert: 'mem>', label: 'mem', desc: 'memory %  (mem>80)' },
  { insert: 'disk>', label: 'disk', desc: 'disk %  (disk<30)' },
  { insert: 'status:', label: 'status:', desc: 'online | offline' },
  { insert: 'kind:', label: 'kind:', desc: 'node | docker | k8s' },
  { insert: 'ns:', label: 'ns:', desc: 'namespace' },
  { insert: 'agent:', label: 'agent:', desc: 'agent version' },
]
const uniq = (f) => [...new Set(props.items.map(f).filter(Boolean))]

// suggestions are context-aware on the token after the last space
const lastToken = computed(() => {
  const parts = text.value.split(/\s+/)
  return parts[parts.length - 1] || ''
})
const suggestions = computed(() => {
  const tok = lastToken.value
  const kv = tok.match(/^(status|kind|ns|agent):(.*)$/i)
  if (kv) {
    const key = kv[1].toLowerCase()
    const val = (kv[2] || '').toLowerCase()
    let vals = []
    if (key === 'status') vals = ['online', 'offline']
    else if (key === 'kind') vals = ['node', 'docker', 'k8s']
    else if (key === 'ns') vals = uniq((s) => s.namespace)
    else if (key === 'agent') vals = uniq((s) => s.agent_version)
    return vals.filter((v) => v.toLowerCase().startsWith(val)).map((v) => ({ insert: `${key}:${v}`, label: `${key}:${v}`, desc: '' }))
  }
  const t = tok.toLowerCase()
  return FIELDS.filter((f) => !t || f.label.toLowerCase().startsWith(t) || f.insert.toLowerCase().startsWith(t))
})

function onFocus() {
  open.value = true
  hi.value = 0
  if (!text.value) text.value = 'name:' // default search field = node name
}
function apply(s) {
  const parts = text.value.split(/\s+/)
  parts[parts.length - 1] = s.insert
  // ':' (field) and trailing '>' (numeric) keep the cursor going; finished values get a space
  const trailing = /[:>]$/.test(s.insert) ? '' : ' '
  text.value = parts.join(' ') + trailing
  hi.value = 0
}
function onKey(e) {
  if (!open.value) return
  const list = suggestions.value
  if (e.key === 'ArrowDown') { e.preventDefault(); hi.value = (hi.value + 1) % list.length }
  else if (e.key === 'ArrowUp') { e.preventDefault(); hi.value = (hi.value - 1 + list.length) % list.length }
  else if (e.key === 'Enter' && list[hi.value]) { e.preventDefault(); apply(list[hi.value]) }
  else if (e.key === 'Escape') { open.value = false }
}
</script>

<template>
  <div class="relative">
    <svg class="absolute left-2.5 top-2.5 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
    <input v-model="text" @focus="onFocus" @blur="open = false" @keydown="onKey"
      placeholder="name: …   try cpu>50 status:online kind:docker"
      class="w-96 rounded-lg border border-line bg-surface2 py-2 pl-8 pr-3 text-sm text-fg outline-none focus:border-accent/50" />
    <div v-if="open && suggestions.length" class="absolute left-0 right-0 z-30 mt-1 max-h-72 overflow-auto rounded-lg border border-line bg-surface2 py-1 shadow-xl">
      <button v-for="(s, i) in suggestions" :key="s.label" @mousedown.prevent="apply(s)"
        class="flex w-full items-center justify-between gap-4 px-3 py-1.5 text-left text-sm" :class="i === hi ? 'bg-accent/15 text-accent' : 'text-fg hover:bg-surface'">
        <span class="tabular-nums">{{ s.label }}</span>
        <span v-if="s.desc" class="text-xs text-faint">{{ s.desc }}</span>
      </button>
    </div>
  </div>
</template>
