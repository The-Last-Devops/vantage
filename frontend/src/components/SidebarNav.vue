<script setup>
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useRoute, useRouter, RouterLink } from 'vue-router'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'

defineProps({
  drawer: { type: Boolean, default: false },
})

const auth = useAuth()
const route = useRoute()
const router = useRouter()

const nsOpen = ref(false)
const nsRef = ref(null)
const namespaces = ref([]) // [{id,name}]

// ---- primary nav (collapsible groups) ----------------------------------
// Parent groups: clicking the parent jumps to its first child; hovering a
// parent reveals its children. When nothing is hovered, the group owning the
// current route stays expanded so you always see where you are.
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const isAdmin = computed(() => !!auth.user?.is_admin)
// Flat top-level nav (no children) — the new attention-first landing + war-room.
const topLinks = [
  { name: 'overview', label: 'Overview', icon: 'dashboard' },
  { name: 'fleet', label: 'Fleet', icon: 'fleet' },
  { name: 'metrics', label: 'Metrics', icon: 'metrics' },
]
const groups = computed(() =>
  [
    {
      key: 'infra', label: 'Infrastructure', icon: 'server',
      // `owns` = extra route names (e.g. detail pages) that belong to this group
      // so it stays highlighted + expanded when you're on them.
      owns: ['system'],
      children: [
        { label: 'All', name: 'systems', owns: ['system'] },
        { label: 'Needs attention', name: 'attention' },
      ],
    },
    {
      key: 'services', label: 'Services', icon: 'service',
      owns: ['monitor', 'monitor-new', 'monitor-edit'],
      children: [
        { label: 'All', name: 'monitors', owns: ['monitor', 'monitor-new', 'monitor-edit'] },
        { label: 'Down', name: 'monitors', down: true },
      ],
    },
    {
      key: 'alert', label: 'Alert', icon: 'alert-triangle',
      owns: ['alert-new', 'alert-edit', 'channel'],
      children: [
        { label: 'Events', name: 'events' },
        { label: 'Rules', name: 'alerts', owns: ['alert-new', 'alert-edit'] },
        { label: 'Notify channel', name: 'notifications', owns: ['channel'] },
      ],
    },
    {
      key: 'settings', label: 'Settings', icon: 'settings',
      owns: ['namespace'],
      children: [
        { label: 'Namespace', name: 'namespaces', owns: ['namespace'] },
        { label: 'Members', name: 'members', admin: true },
        { label: 'Audit', name: 'audit', admin: true },
        { label: 'Data & retention', name: 'data', admin: true },
        { label: 'Backup', name: 'backup', admin: true },
        { label: 'API tokens', name: 'tokens' },
        { label: 'SSH keys', name: 'ssh-keys' },
        { label: 'About', name: 'about' },
      ],
    },
  ].map((g) => ({ ...g, children: g.children.filter((c) => !c.admin || isAdmin.value) })),
)
// Carry the namespace selection (?ns) onto every nav link so it never drops.
const childTo = (c) => {
  const query = { ...nsq.value }
  if (c.down) query.status = 'down'
  return { name: c.name, query }
}
const childActive = (c) => {
  // Detail / editor sub-routes (e.g. /channel/:id) highlight their owning child.
  if (c.owns && c.owns.includes(route.name)) return true
  if (route.name !== c.name) return false
  if (c.name === 'monitors') return (route.query.status === 'down') === !!c.down
  return true
}
const groupActive = (g) => g.children.some((c) => childActive(c)) || (g.owns || []).includes(route.name)
// A group is expanded when it's the active route's group (always open) or when
// the user has clicked its chevron open. No hover — opening is an explicit click.
const openKey = ref(null)
const expanded = (g) => openKey.value === g.key || groupActive(g)
function openGroup(g) {
  const first = g.children[0]
  if (first && !childActive(first)) router.push(childTo(first))
}

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
// close the namespace dropdown on any click outside it (e.g. on the fleet table)
function onDocClick(e) { if (nsOpen.value && nsRef.value && !nsRef.value.contains(e.target)) nsOpen.value = false }
onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))

async function logout() { await auth.logout(); router.push({ name: 'login' }) }
</script>

<template>
  <aside :class="['fixed inset-y-0 left-0 z-40 flex h-[100dvh] w-60 shrink-0 flex-col border-r border-line bg-surface transition-transform md:sticky md:top-0 md:translate-x-0', drawer ? '' : '-translate-x-full']">
    <RouterLink :to="{ name: 'systems', query: nsq }" class="flex items-center gap-2.5 px-5 py-4 transition-opacity hover:opacity-80" v-tip="`Home`">
      <span class="vantage-logo grid h-6 w-6 place-items-center rounded-md">
        <svg viewBox="0 0 64 64" class="h-[15px] w-[15px]" fill="none" stroke="#08231F" stroke-width="5.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M7 43 L21 21 L35 43"/>
          <rect x="42" y="37.5" width="14" height="6" rx="1.8" fill="#08231F" stroke="none"/>
        </svg>
      </span>
      <span class="text-base font-semibold tracking-tight text-fg">Vantage</span>
    </RouterLink>

    <!-- nav -->
    <nav class="flex-1 space-y-0.5 overflow-y-auto px-3 py-2">
      <!-- flat top-level entries (no children): Overview + Fleet -->
      <RouterLink v-for="t in topLinks" :key="t.name" :to="{ name: t.name, query: nsq }"
        class="relative flex items-center gap-2.5 rounded-lg py-2 pl-3 pr-1 text-sm transition"
        :class="route.name === t.name ? 'bg-surface2 font-semibold text-fg' : 'font-medium text-muted hover:bg-surface2 hover:text-fg'">
        <span v-if="route.name === t.name" class="absolute -left-3 top-1.5 bottom-1.5 w-[3px] rounded-r bg-accent"></span>
        <VIcon :name="t.icon" :size="18" class="shrink-0" /><span class="flex-1 truncate">{{ t.label }}</span>
      </RouterLink>
      <div v-for="g in groups" :key="g.key">
        <div class="relative flex items-center rounded-lg text-sm transition"
          :class="groupActive(g) ? 'bg-surface2 font-semibold text-fg' : 'font-medium text-fg hover:bg-surface2'">
          <span v-if="groupActive(g)" class="absolute -left-3 top-1.5 bottom-1.5 w-[3px] rounded-r bg-accent"></span>
          <button @click="openGroup(g)" class="flex min-w-0 flex-1 items-center gap-2.5 py-2 pl-3 pr-1 text-left"
            :class="groupActive(g) ? 'text-fg' : 'text-muted hover:text-fg'">
            <VIcon :name="g.icon" :size="18" class="shrink-0" />
            <span class="flex-1 truncate">{{ g.label }}</span>
          </button>
          <!-- the submenu opens only on clicking this chevron (no hover) -->
          <button @click.stop="openKey = openKey === g.key ? null : g.key"
            class="shrink-0 px-3 py-2 text-faint hover:text-fg" v-tip="expanded(g) ? 'Hide submenu' : 'Show submenu'" aria-label="Toggle submenu">
            <svg class="h-3.5 w-3.5 transition-transform" :class="expanded(g) ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg>
          </button>
        </div>
        <div v-show="expanded(g)" class="mt-0.5 space-y-0.5">
          <RouterLink v-for="c in g.children" :key="c.label + (c.down ? '-down' : '')" :to="childTo(c)"
            class="flex items-center rounded-lg py-1.5 pl-10 pr-3 text-sm transition hover:bg-surface2 hover:text-fg"
            :class="childActive(c) ? '!bg-accent/10 font-semibold !text-accent' : 'font-medium text-muted'">{{ c.label }}</RouterLink>
        </div>
      </div>
    </nav>

    <!-- namespace multi-select — at the bottom so its dropdown opens upward
         and never covers the nav links -->
    <div ref="nsRef" class="relative border-t border-line px-3 py-2">
      <div class="px-1 pb-1 text-[11px] uppercase tracking-wider text-faint">Namespace</div>
      <button @click="nsOpen = !nsOpen"
        class="flex w-full items-center justify-between gap-2 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50">
        <span class="flex min-w-0 items-center gap-2"><span class="h-2 w-2 shrink-0 rounded-full bg-accent"></span><span class="truncate">{{ nsLabel }}</span></span>
        <svg class="h-4 w-4 shrink-0 text-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m18 15-6-6-6 6"/></svg>
      </button>
      <div v-if="nsOpen" class="absolute bottom-full left-3 right-3 z-30 mb-1 max-h-72 overflow-y-auto rounded-lg border border-line bg-surface2 py-1 shadow-xl">
        <button @click="toggleAllNs()" class="flex w-full items-center gap-2.5 border-b border-line px-3 py-2 text-left text-sm hover:bg-surface" :class="allChecked ? 'text-accent' : 'text-muted'">
          <span class="grid h-4 w-4 place-items-center rounded border" :class="allChecked ? 'border-accent bg-accent' : 'border-line'"></span>All namespaces
        </button>
        <button v-for="n in namespaces" :key="n.id" @click="toggleNs(n.name)"
          class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface" :class="nsChecked(n.name) ? 'text-fg' : 'text-muted'">
          <span class="grid h-4 w-4 place-items-center rounded border" :class="nsChecked(n.name) ? 'border-accent bg-accent' : 'border-line'"></span>{{ n.name }}
        </button>
      </div>
    </div>

    <div class="border-t border-line p-3">
      <div class="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
        <span class="grid h-8 w-8 place-items-center rounded-full bg-surface2 text-xs text-accent">{{ (auth.user?.email || '?').slice(0,2).toUpperCase() }}</span>
        <div class="min-w-0 flex-1"><div class="truncate text-sm text-fg">{{ auth.user?.email }}</div><div class="text-[11px] text-faint">{{ auth.user?.is_admin ? 'Admin' : 'Member' }}</div></div>
        <button @click="logout" v-tip="`Logout`" class="text-muted hover:text-accent"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/></svg></button>
      </div>
    </div>
  </aside>
</template>
