<script setup>
import { ref, onMounted, onBeforeUnmount, computed, watch } from 'vue'
import { useRoute, useRouter, RouterLink } from 'vue-router'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'
import { useUi } from '../stores/ui'

const props = defineProps({
  title: { type: String, default: '' },
  hideTitle: { type: Boolean, default: false },
})

const auth = useAuth()
const ui = useUi()
const route = useRoute()
const router = useRouter()

const drawer = ref(false)
const nsOpen = ref(false)
const nsRef = ref(null)
const namespaces = ref([]) // [{id,name}]

// ---- primary nav (collapsible groups) ----------------------------------
// Parent groups: clicking the parent jumps to its first child; hovering a
// parent reveals its children. When nothing is hovered, the group owning the
// current route stays expanded so you always see where you are.
const nsq = computed(() => (route.query.ns ? { ns: route.query.ns } : {}))
const isAdmin = computed(() => !!auth.user?.is_admin)
const groups = computed(() =>
  [
    {
      key: 'infra', label: 'Infra',
      children: [
        { label: 'All', name: 'systems' },
        { label: 'Needs attention', name: 'attention' },
      ],
    },
    {
      key: 'services', label: 'Services',
      children: [
        { label: 'All', name: 'monitors' },
        { label: 'Down', name: 'monitors', down: true },
      ],
    },
    {
      key: 'alert', label: 'Alert',
      children: [
        { label: 'All', name: 'alerts' },
        { label: 'Notify channel', name: 'notifications' },
      ],
    },
    {
      key: 'settings', label: 'Settings',
      children: [
        { label: 'Namespace', name: 'namespaces' },
        { label: 'Members', name: 'members', admin: true },
        { label: 'Audit', name: 'audit', admin: true },
        { label: 'Data & retention', name: 'data', admin: true },
        { label: 'About', name: 'about' },
      ],
    },
  ].map((g) => ({ ...g, children: g.children.filter((c) => !c.admin || isAdmin.value) })),
)
const NS_QUERY_PAGES = new Set(['systems', 'attention', 'monitors'])
const childTo = (c) => {
  const query = NS_QUERY_PAGES.has(c.name) ? { ...nsq.value } : {}
  if (c.down) query.status = 'down'
  return { name: c.name, query }
}
const childActive = (c) => {
  if (route.name !== c.name) return false
  if (c.name === 'monitors') return (route.query.status === 'down') === !!c.down
  return true
}
const groupActive = (g) => g.children.some((c) => childActive(c))
const hovered = ref(null)
const expanded = (g) => hovered.value === g.key || (hovered.value === null && groupActive(g))
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

// browser tab title follows the page (e.g. "docker-01 — Last Monitor")
watch(() => props.title, (t) => { document.title = t ? `${t} — Last Monitor` : 'Last Monitor' }, { immediate: true })
</script>

<template>
  <div class="flex min-h-screen">
    <div v-if="drawer" class="fixed inset-0 z-30 bg-black/60 md:hidden" @click="drawer = false"></div>

    <!-- sidebar -->
    <aside :class="['fixed inset-y-0 left-0 z-40 flex h-screen w-60 shrink-0 flex-col border-r border-line bg-surface transition-transform md:sticky md:top-0 md:translate-x-0', drawer ? '' : '-translate-x-full']">
      <div class="flex items-center gap-2.5 px-5 py-4">
        <span class="lm-logo inline-block h-6 w-6 rounded-md"></span>
        <span class="text-base font-semibold tracking-tight text-fg">Last Monitor</span>
      </div>

      <!-- nav -->
      <nav class="flex-1 space-y-0.5 overflow-y-auto px-3 py-2">
        <div v-for="g in groups" :key="g.key" @mouseenter="hovered = g.key" @mouseleave="hovered = null">
          <button @click="openGroup(g)"
            class="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition hover:bg-surface2"
            :class="groupActive(g) ? 'font-medium text-fg' : 'text-muted hover:text-fg'">
            <svg v-if="g.key === 'infra'" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>
            <svg v-else-if="g.key === 'services'" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
            <svg v-else-if="g.key === 'alert'" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>
            <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/></svg>
            <span class="flex-1 text-left">{{ g.label }}</span>
            <svg class="h-3.5 w-3.5 shrink-0 text-faint transition-transform" :class="expanded(g) ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg>
          </button>
          <div v-show="expanded(g)" class="mt-0.5 space-y-0.5">
            <RouterLink v-for="c in g.children" :key="c.label + (c.down ? '-down' : '')" :to="childTo(c)"
              class="flex items-center rounded-lg py-1.5 pl-10 pr-3 text-sm transition hover:bg-surface2 hover:text-fg"
              :class="childActive(c) ? '!bg-accent/10 font-medium !text-accent' : 'text-muted'">{{ c.label }}</RouterLink>
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
          <button @click="logout" title="Logout" class="text-muted hover:text-accent"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"/></svg></button>
        </div>
      </div>
    </aside>

    <!-- main -->
    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex items-center justify-between border-b border-line bg-surface/60 px-4 py-3 backdrop-blur sm:px-6">
        <div class="flex items-center gap-3">
          <button @click="drawer = true" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent md:hidden">
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
          </button>
          <h1 v-if="title && !hideTitle" class="text-lg font-semibold text-fg">{{ title }}</h1>
          <slot name="title-after" />
        </div>
        <div class="flex items-center gap-3">
          <slot name="header" />
          <button @click="ui.toggleTheme()" title="Toggle theme" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent">
          <svg v-if="!ui.light" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
          <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
          </button>
        </div>
      </header>
      <main class="flex-1 p-4 sm:p-6"><slot /></main>
    </div>
  </div>
</template>

<style scoped>
.lm-soon { display: flex; align-items: center; gap: 0.625rem; border-radius: 0.5rem; padding: 0.5rem 0.75rem; font-size: 0.875rem; color: rgb(var(--faint)); cursor: not-allowed; }
.lm-soon-pill { margin-left: auto; border-radius: 0.25rem; background: rgb(var(--surface2)); padding: 0.05rem 0.4rem; font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; }
</style>
