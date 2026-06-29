<script setup>
import { ref, computed } from 'vue'

// Composes one filter token at a time. Enter (or picking a complete suggestion)
// emits 'add' with the token; the parent keeps the committed filters as chips.
const props = defineProps({ items: { type: Array, default: () => [] } })
const emit = defineEmits(['add'])

const text = ref('')
const open = ref(false)
const hi = ref(0)

const FIELDS = [
  { insert: 'name:', label: 'name:', desc: 'node name (default)' },
  { insert: 'cpu', label: 'cpu', desc: 'cpu %' },
  { insert: 'mem', label: 'mem', desc: 'memory %' },
  { insert: 'disk', label: 'disk', desc: 'disk %' },
  { insert: 'status:', label: 'status:', desc: 'online | offline' },
  { insert: 'kind:', label: 'kind:', desc: 'node | docker | k8s' },
  { insert: 'ns:', label: 'ns:', desc: 'namespace' },
  { insert: 'agent:', label: 'agent:', desc: 'agent version' },
]
const OPS = ['=', '>', '<', '>=', '<=']
const uniq = (f) => [...new Set(props.items.map(f).filter(Boolean))]
// a token still needs more input (don't commit on click/Enter): trailing : or
// comparison operator, or a bare numeric field awaiting an operator
const needsMore = (ins) => /[:=<>]$/.test(ins) || /^(cpu|mem|disk)$/i.test(ins)

const suggestions = computed(() => {
  const tok = text.value.trim()
  // numeric field chosen but no operator yet → offer comparison operators
  const nf = tok.match(/^(cpu|mem|disk)$/i)
  if (nf) {
    const f = nf[1].toLowerCase()
    return OPS.map((op) => ({ insert: `${f}${op}`, label: `${f} ${op}`, desc: 'compare' }))
  }
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

function onFocus() { open.value = true; hi.value = 0 }
function reset() { text.value = ''; open.value = false; hi.value = 0 }
// don't commit a field/operator that still lacks a value
function commit() {
  const t = text.value.trim()
  if (!t || /^(cpu|mem|disk)(>=|<=|>|<|=)?$/i.test(t)) return
  emit('add', t); reset()
}
// click a suggestion: if it still needs input (field, operator, colon) keep
// composing in the box; otherwise it's a complete token — commit it
function pick(s) {
  if (needsMore(s.insert)) { text.value = s.insert; open.value = true; hi.value = 0 }
  else { emit('add', s.insert); reset() }
}
function onKey(e) {
  const list = suggestions.value
  if (e.key === 'ArrowDown') { e.preventDefault(); open.value = true; hi.value = (hi.value + 1) % list.length }
  else if (e.key === 'ArrowUp') { e.preventDefault(); hi.value = (hi.value - 1 + list.length) % list.length }
  else if (e.key === 'Enter') {
    e.preventDefault()
    const s = open.value ? list[hi.value] : null
    // while still composing, Enter takes the highlighted suggestion; else commit
    if (s && needsMore(text.value.trim())) pick(s)
    else commit()
  } else if (e.key === 'Escape') { open.value = false }
}
</script>

<template>
  <div class="relative">
    <svg class="absolute left-2.5 top-2.5 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
    <input v-model="text" @focus="onFocus" @blur="open = false" @keydown="onKey"
      placeholder="Filter…  cpu>50  ns:prod  web*  — Enter to add"
      class="w-72 rounded-lg border border-line bg-surface2 py-2 pl-8 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none sm:w-96" />
    <div v-if="open && suggestions.length" class="absolute left-0 right-0 z-30 mt-1 max-h-72 overflow-auto rounded-lg border border-line bg-surface2 py-1 shadow-xl">
      <button v-for="(s, i) in suggestions" :key="s.label" @mousedown.prevent="pick(s)"
        class="flex w-full items-center justify-between gap-4 px-3 py-1.5 text-left text-sm" :class="i === hi ? 'bg-accent/15 text-accent' : 'text-fg hover:bg-surface'">
        <span class="font-mono tabular-nums">{{ s.label }}</span>
        <span v-if="s.desc" class="text-xs text-faint">{{ s.desc }}</span>
      </button>
    </div>
  </div>
</template>
