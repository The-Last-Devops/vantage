<script setup>
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { useUi } from '../stores/ui'
import { useVersion } from '../stores/version'
import NamespacePicker from './NamespacePicker.vue'
import AccountMenu from './AccountMenu.vue'

defineProps({
  title: { type: String, default: '' },
  hideTitle: { type: Boolean, default: false },
  // Breadcrumb shown in the header bar instead of `title` (saves a row vs an
  // in-page breadcrumb). Array of { label, to? }; the last item is the current page.
  breadcrumb: { type: Array, default: null },
})
defineEmits(['open-drawer'])

const ui = useUi()
const ver = useVersion()
const router = useRouter()
onMounted(() => ver.ensureLoaded())

// ⌘K / Ctrl-K focuses the search affordance. The full command palette is a
// follow-up; for now Enter jumps to the Systems list so it isn't a dead end.
const searchRef = ref(null)
const searchQuery = ref('')
function onKey(e) {
  if ((e.metaKey || e.ctrlKey) && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault()
    searchRef.value?.focus()
  }
}
function onSearchEnter() {
  router.push({ name: 'systems' })
  searchQuery.value = ''
  searchRef.value?.blur()
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <header class="flex h-[56px] items-center gap-3 border-b border-line2 bg-head px-4">
    <!-- left: hamburger + breadcrumb / title -->
    <div class="flex min-w-0 items-center gap-3">
      <button @click="$emit('open-drawer')" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent md:hidden">
        <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
      </button>
      <nav v-if="breadcrumb" class="flex min-w-0 items-center gap-1.5 text-sm">
        <template v-for="(b, i) in breadcrumb" :key="i">
          <RouterLink v-if="b.to" :to="b.to" class="shrink-0 text-muted hover:text-accent">{{ i === 0 ? '‹ ' : '' }}{{ b.label }}</RouterLink>
          <span v-else class="truncate font-semibold text-fg">{{ b.label }}</span>
          <span v-if="i < breadcrumb.length - 1" class="shrink-0 text-faint">/</span>
        </template>
      </nav>
      <h1 v-else-if="title && !hideTitle" class="text-lg font-semibold text-fg">{{ title }}</h1>
      <slot name="title-after" />
    </div>

    <!-- center: search affordance (⌘K) -->
    <div class="hidden flex-1 justify-center md:flex">
      <div class="flex w-full max-w-[420px] items-center gap-2 rounded-lg border border-line2 bg-surface2 px-3 py-1.5 focus-within:border-accent/50">
        <VIcon name="search" :size="15" class="shrink-0 text-faint" />
        <input ref="searchRef" v-model="searchQuery" @keydown.enter="onSearchEnter"
          type="text" placeholder="Search hosts, services, commands…"
          class="min-w-0 flex-1 bg-transparent text-sm text-fg placeholder:text-faint focus:outline-none" />
        <kbd class="hidden shrink-0 rounded border border-line2 bg-surface px-1.5 py-0.5 text-[10px] font-medium text-faint lg:inline-block">⌘K</kbd>
      </div>
    </div>

    <!-- right cluster -->
    <div class="ml-auto flex items-center gap-2">
      <!-- page-supplied actions/header still render here, before the global controls -->
      <slot name="actions" />
      <slot name="header" />

      <NamespacePicker />

      <span class="hidden items-center gap-1.5 text-xs text-muted sm:inline-flex">
        <span class="h-1.5 w-1.5 rounded-full bg-ok"></span><span class="hidden lg:inline">live</span>
      </span>

      <span class="h-5 w-px bg-line2"></span>

      <RouterLink :to="{ name: 'events' }" v-tip="`Alerts`"
        class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent">
        <VIcon name="bell" :size="16" />
      </RouterLink>

      <button @click="ui.toggleTheme()" v-tip="`Toggle theme`" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent">
        <svg v-if="!ui.light" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
        <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
      </button>

      <!-- build version → About; green = up to date / unknown, amber = newer release out -->
      <RouterLink v-if="ver.current" :to="{ name: 'about' }"
        v-tip="ver.isOutdated ? `Update available: ${ver.latestTag} — you have v${ver.current}` : `You're on v${ver.current}`"
        class="hidden items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-colors sm:inline-flex"
        :class="ver.isOutdated
          ? 'border-amber-400/40 bg-amber-400/10 text-amber-400 hover:bg-amber-400/20'
          : 'border-emerald-400/40 bg-emerald-400/10 text-emerald-400 hover:bg-emerald-400/20'">
        <span class="h-1.5 w-1.5 rounded-full" :class="ver.isOutdated ? 'bg-amber-400' : 'bg-emerald-400'"></span>
        v{{ ver.current }}
      </RouterLink>

      <AccountMenu />
    </div>
  </header>
</template>
