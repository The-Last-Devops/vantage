<script setup>
import { ref, onMounted, computed } from 'vue'
import { useRouter, RouterLink } from 'vue-router'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'
import { useUi } from '../stores/ui'

defineProps({ title: { type: String, default: '' } })

const auth = useAuth()
const ui = useUi()
const router = useRouter()

const drawer = ref(false)
const nsOpen = ref(false)
const namespaces = ref([]) // [{id,name}]

const nsNames = computed(() => namespaces.value.map((n) => n.name))
const nsLabel = computed(() => {
  if (!ui.nsTouched) return 'All namespaces'
  const n = ui.selectedNs.size
  if (n === 0) return 'No namespace'
  if (n === nsNames.value.length) return 'All namespaces'
  if (n === 1) return [...ui.selectedNs][0]
  return `${n} namespaces`
})
const nsChecked = (name) => !ui.nsTouched || ui.selectedNs.has(name)
const allChecked = computed(() => !ui.nsTouched || ui.selectedNs.size === nsNames.value.length)

onMounted(async () => {
  try { namespaces.value = await api.get('/api/namespaces') } catch { namespaces.value = [] }
})

async function logout() { await auth.logout(); router.push({ name: 'login' }) }
</script>

<template>
  <div class="flex min-h-screen">
    <div v-if="drawer" class="fixed inset-0 z-30 bg-black/60 md:hidden" @click="drawer = false"></div>

    <!-- sidebar -->
    <aside :class="['fixed inset-y-0 left-0 z-40 flex w-60 shrink-0 flex-col border-r border-line bg-surface transition-transform md:static md:translate-x-0', drawer ? '' : '-translate-x-full']">
      <div class="flex items-center gap-2.5 px-5 py-4">
        <span class="inline-block h-6 w-6 rounded-md bg-accent shadow-[0_0_18px_-4px_rgb(var(--accent))]"></span>
        <span class="text-base font-semibold tracking-tight text-fg">last-monitor</span>
      </div>

      <!-- namespace multi-select -->
      <div class="relative px-3 pb-2">
        <button @click="nsOpen = !nsOpen"
          class="flex w-full items-center justify-between gap-2 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50">
          <span class="flex min-w-0 items-center gap-2"><span class="h-2 w-2 shrink-0 rounded-full bg-accent"></span><span class="truncate">{{ nsLabel }}</span></span>
          <svg class="h-4 w-4 shrink-0 text-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>
        </button>
        <div v-if="nsOpen" class="absolute left-3 right-3 z-30 mt-1 overflow-hidden rounded-lg border border-line bg-surface2 py-1 shadow-xl">
          <button @click="ui.toggleAllNs(nsNames)" class="flex w-full items-center gap-2.5 border-b border-line px-3 py-2 text-left text-sm hover:bg-surface" :class="allChecked ? 'text-accent' : 'text-muted'">
            <span class="grid h-4 w-4 place-items-center rounded border" :class="allChecked ? 'border-accent bg-accent' : 'border-line'"></span>All namespaces
          </button>
          <button v-for="n in namespaces" :key="n.id" @click="ui.toggleNs(n.name, nsNames)"
            class="flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm hover:bg-surface" :class="nsChecked(n.name) ? 'text-fg' : 'text-muted'">
            <span class="grid h-4 w-4 place-items-center rounded border" :class="nsChecked(n.name) ? 'border-accent bg-accent' : 'border-line'"></span>{{ n.name }}
          </button>
        </div>
      </div>

      <!-- nav -->
      <nav class="flex-1 space-y-1 overflow-y-auto px-3 py-2">
        <RouterLink to="/" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg" active-class="!bg-accent/10 font-medium !text-accent" exact-active-class="!bg-accent/10 font-medium !text-accent">
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>
          Systems
        </RouterLink>
        <div class="px-3 pb-1 pt-4 text-[11px] uppercase tracking-wider text-faint">Manage</div>
        <a href="#" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7h18M3 12h18M3 17h18"/></svg>Namespaces</a>
        <a href="#" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 6h16M4 12h16M4 18h10"/></svg>Systems</a>
        <a href="#" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>Notifications</a>
        <a href="#" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>Members</a>
        <a href="#" class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-muted transition hover:bg-surface2 hover:text-fg"><svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/></svg>Data &amp; retention</a>
      </nav>

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
          <h1 class="text-lg font-semibold text-fg">{{ title }}</h1>
        </div>
        <button @click="ui.toggleTheme()" title="Toggle theme" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent">
          <svg v-if="!ui.light" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
          <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
        </button>
      </header>
      <main class="flex-1 p-4 sm:p-6"><slot /></main>
    </div>
  </div>
</template>
