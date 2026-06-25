<script setup>
// A Rancher-style data table: sortable headers, optional row selection with a
// bulk-action toolbar, a built-in filter box, hover rows, and themed contrast.
// Custom cells via scoped slots named `cell-<key>`; the toolbar's left side via
// the `toolbar` slot (replaced by the `bulk` slot when rows are selected).
import { ref, computed, watchEffect } from 'vue'

const props = defineProps({
  // [{ key, label, sortable, align: 'left'|'right'|'center', width, mono, nowrap, class }]
  columns: { type: Array, required: true },
  rows: { type: Array, default: () => [] },
  rowKey: { type: Function, default: (r) => r.id },
  selectable: { type: Boolean, default: false },
  clickable: { type: Boolean, default: false }, // emit row-click + cursor-pointer
  filterable: { type: Boolean, default: true },
  filterKeys: { type: Array, default: () => [] }, // [] = all column keys
  filterPlaceholder: { type: String, default: 'Filter…' },
  loading: { type: Boolean, default: false },
  empty: { type: String, default: 'Nothing here yet.' },
  emptyFiltered: { type: String, default: 'Nothing matches your filter.' },
  initialSort: { type: Object, default: null }, // { key, dir: 'asc'|'desc' }
})
const emit = defineEmits(['row-click'])
const selected = defineModel('selected', { default: () => [] }) // array of row keys

// ---- filter ----
const q = ref('')
const keys = computed(() => (props.filterKeys.length ? props.filterKeys : props.columns.map((c) => c.key)))
const filtered = computed(() => {
  const n = q.value.trim().toLowerCase()
  if (!n) return props.rows
  return props.rows.filter((r) => keys.value.map((k) => r[k]).filter((v) => v != null).join(' ').toLowerCase().includes(n))
})

// ---- sort ----
const sortKey = ref(props.initialSort?.key ?? null)
const sortDir = ref(props.initialSort?.dir ?? 'asc')
function toggleSort(c) {
  if (!c.sortable) return
  if (sortKey.value === c.key) sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  else { sortKey.value = c.key; sortDir.value = 'asc' }
}
const sorted = computed(() => {
  if (!sortKey.value) return filtered.value
  const k = sortKey.value
  const d = sortDir.value === 'asc' ? 1 : -1
  return [...filtered.value].sort((a, b) => {
    const x = a[k], y = b[k]
    if (x == null) return 1
    if (y == null) return -1
    return (typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y))) * d
  })
})

// ---- selection ----
const allKeys = computed(() => sorted.value.map(props.rowKey))
const allSel = computed(() => allKeys.value.length > 0 && allKeys.value.every((k) => selected.value.includes(k)))
const someSel = computed(() => selected.value.length > 0 && !allSel.value)
const selectedRows = computed(() => sorted.value.filter((r) => selected.value.includes(props.rowKey(r))))
function toggleAll() { selected.value = allSel.value ? [] : [...allKeys.value] }
function toggleRow(k) { selected.value = selected.value.includes(k) ? selected.value.filter((x) => x !== k) : [...selected.value, k] }
function clearSel() { selected.value = [] }

const headCb = ref(null)
watchEffect(() => { if (headCb.value) headCb.value.indeterminate = someSel.value })

const colCount = computed(() => props.columns.length + (props.selectable ? 1 : 0))
const alignCls = (a) => (a === 'right' ? 'text-right' : a === 'center' ? 'text-center' : 'text-left')
</script>

<template>
  <div>
    <!-- toolbar: bulk actions are ALWAYS visible (disabled until a row is selected,
         Rancher-style) so the feature is discoverable. -->
    <div v-if="$slots.toolbar || $slots.bulk || filterable"
      class="flex flex-wrap items-center gap-2.5 rounded-t-xl border border-b-0 border-line bg-surface px-3.5 py-2.5">
      <slot name="toolbar" />
      <slot name="bulk" :selected="selectedRows" :count="selected.length" :disabled="selected.length === 0" :clear="clearSel" />
      <span v-if="selectable && selected.length" class="text-sm font-semibold text-accent">{{ selected.length }} selected</span>
      <button v-if="selectable && selected.length" @click="clearSel" class="text-xs text-faint hover:text-fg">Clear</button>
      <span class="flex-1"></span>
      <div v-if="filterable" class="relative">
        <svg class="absolute left-2.5 top-2 h-4 w-4 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
        <input v-model="q" :placeholder="filterPlaceholder" class="w-56 rounded-lg border border-line-strong bg-bg py-1.5 pl-8 pr-3 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" />
      </div>
    </div>

    <!-- table -->
    <div class="overflow-x-auto rounded-b-xl border border-line bg-surface">
      <table class="w-full border-collapse">
        <thead>
          <tr>
            <th v-if="selectable" class="w-9 border-b border-line-strong bg-headbg px-3 py-2.5">
              <input ref="headCb" type="checkbox" :checked="allSel" @change="toggleAll" class="h-[15px] w-[15px] cursor-pointer align-middle accent-[rgb(var(--accent))]" />
            </th>
            <th v-for="c in columns" :key="c.key" @click="toggleSort(c)"
              class="select-none border-b border-line-strong bg-headbg px-3.5 py-2.5 text-xs font-semibold text-muted"
              :class="[alignCls(c.align), c.sortable ? 'cursor-pointer hover:text-fg' : '', c.nowrap !== false ? 'whitespace-nowrap' : '']"
              :style="c.width ? { width: c.width } : null">
              {{ c.label }}<span v-if="c.sortable && sortKey === c.key" class="ml-1 text-accent">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="loading"><td :colspan="colCount" class="px-4 py-12 text-center text-sm text-muted">Loading…</td></tr>
          <tr v-else-if="!sorted.length"><td :colspan="colCount" class="px-4 py-12 text-center text-sm text-muted">{{ q.trim() ? emptyFiltered : empty }}</td></tr>
          <tr v-for="row in sorted" :key="rowKey(row)"
            class="border-b border-line last:border-0 hover:bg-hover"
            :class="[selected.includes(rowKey(row)) ? 'bg-accent/[0.07]' : '', clickable ? 'cursor-pointer' : '']"
            @click="clickable && emit('row-click', row)">
            <td v-if="selectable" class="px-3 py-2.5" @click.stop>
              <input type="checkbox" :checked="selected.includes(rowKey(row))" @change="toggleRow(rowKey(row))" class="h-[15px] w-[15px] cursor-pointer align-middle accent-[rgb(var(--accent))]" />
            </td>
            <td v-for="c in columns" :key="c.key" class="px-3.5 py-2.5 text-sm text-fg"
              :class="[alignCls(c.align), c.mono ? 'font-mono tabular-nums' : '', c.nowrap !== false ? 'whitespace-nowrap' : '', c.class]">
              <slot :name="`cell-${c.key}`" :row="row" :value="row[c.key]">{{ row[c.key] ?? '—' }}</slot>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
