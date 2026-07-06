<script setup>
import { ref, computed } from 'vue'
import { useRoute, useRouter, RouterLink } from 'vue-router'
import { useAuth } from '../stores/auth'

defineProps({
  drawer: { type: Boolean, default: false },
})

const auth = useAuth()
const route = useRoute()
const router = useRouter()

// ---- primary nav (collapsible groups) ----------------------------------
// Parent groups: clicking the parent jumps to its first child; hovering a
// parent reveals its children. When nothing is hovered, the group owning the
// current route stays expanded so you always see where you are.
const nsq = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))
const isAdmin = computed(() => !!auth.user?.is_admin)
// Flat top-level nav (no children). Fleet + Metrics are temporarily hidden from the
// nav (routes/code kept, reachable by URL) — they overlapped and read as noise on
// small fleets; revisit once there's a clear, data-rich need. Overview is the dashboard.
const topLinks = [
  { name: 'overview', label: 'Overview', icon: 'dashboard' },
]
const groups = computed(() =>
  [
    {
      key: 'infra', label: 'Infrastructure', icon: 'server',
      // `owns` = extra route names (e.g. detail pages) that belong to this group
      // so it stays highlighted + expanded when you're on them.
      owns: ['system'],
      children: [
        { label: 'All', name: 'systems', icon: 'fleet', owns: ['system'] },
        { label: 'Issues', name: 'attention', icon: 'alert-triangle' },
      ],
    },
    {
      key: 'services', label: 'Services', icon: 'service',
      owns: ['monitor', 'monitor-new', 'monitor-edit'],
      children: [
        { label: 'All', name: 'monitors', icon: 'service', owns: ['monitor', 'monitor-new', 'monitor-edit'] },
        { label: 'Down', name: 'monitors', icon: 'wifi-off', down: true },
      ],
    },
    {
      key: 'alert', label: 'Alert', icon: 'flame',
      owns: ['alert-new', 'alert-edit', 'channel'],
      children: [
        { label: 'Events', name: 'events', icon: 'pulse' },
        { label: 'Rules', name: 'alerts', icon: 'sliders', owns: ['alert-new', 'alert-edit'] },
        { label: 'Notify channel', name: 'notifications', icon: 'bell', owns: ['channel'] },
      ],
    },
    {
      key: 'settings', label: 'Settings', icon: 'settings',
      owns: ['workspace'],
      children: [
        { label: 'Workspace', name: 'workspaces', icon: 'globe', owns: ['workspace'] },
        { label: 'Members', name: 'members', icon: 'user', admin: true },
        { label: 'Audit', name: 'audit', icon: 'logs', admin: true },
        { label: 'Data & retention', name: 'data', icon: 'disk', admin: true },
        { label: 'Backup', name: 'backup', icon: 'archive', admin: true },
        { label: 'Security', name: 'security', icon: 'shield' },
        { label: 'API tokens', name: 'tokens', icon: 'key' },
        { label: 'SSH keys', name: 'ssh-keys', icon: 'ssh' },
        { label: 'About', name: 'about', icon: 'info' },
      ],
    },
  ].map((g) => ({ ...g, children: g.children.filter((c) => !c.admin || isAdmin.value) })),
)
// Carry the workspace selection (?ws) onto every nav link so it never drops.
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
</script>

<template>
  <aside :class="['fixed inset-y-0 left-0 z-40 flex h-[100dvh] w-60 shrink-0 flex-col border-r border-line bg-surface transition-transform md:sticky md:top-0 md:translate-x-0', drawer ? '' : '-translate-x-full']">
    <RouterLink :to="{ name: 'systems', query: nsq }" class="flex items-center gap-2.5 px-5 py-4 transition-opacity hover:opacity-80" v-tip="`Home`">
      <VLogo :size="24" />
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
            class="flex items-center gap-2.5 rounded-lg py-1.5 pl-5 pr-3 text-sm transition hover:bg-surface2 hover:text-fg"
            :class="childActive(c) ? '!bg-accent/10 font-semibold !text-accent' : 'font-medium text-muted'">
            <VIcon v-if="c.icon" :name="c.icon" :size="16" class="shrink-0 opacity-80" /><span class="flex-1 truncate">{{ c.label }}</span>
          </RouterLink>
        </div>
      </div>
    </nav>
  </aside>
</template>
