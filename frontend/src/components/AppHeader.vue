<script setup>
// Canonical top bar (design-system header.html): identity + status ONLY.
// Left = breadcrumb / page name. Right = a compact ⌘K jump, the namespace switcher,
// live status, alerts, theme, version, account. Page-scoped search + action buttons do
// NOT live here — they go in the page bar (AppShell) below, scoped to the current page.
import { onMounted, onBeforeUnmount, computed } from 'vue'
import { RouterLink, useRouter, useRoute } from 'vue-router'
import { useUi } from '../stores/ui'
import { useVersion } from '../stores/version'
import NamespacePicker from './NamespacePicker.vue'
import AccountMenu from './AccountMenu.vue'

const props = defineProps({
  title: { type: String, default: '' },
  hideTitle: { type: Boolean, default: false }, // deprecated no-op; header always names the page
  breadcrumb: { type: Array, default: null }, // [{ label, to? }] — last item is current
})
defineEmits(['open-drawer'])

const ui = useUi()
const ver = useVersion()
const router = useRouter()
const route = useRoute()
onMounted(() => ver.ensureLoaded())

// Header is never blank: explicit title → route meta.title.
const headTitle = computed(() => props.title || route.meta?.title || '')

// ⌘K global jump. Full command palette is a follow-up; for now it jumps to the host list.
function jump() {
  router.push({ name: 'systems', query: route.query.ns ? { ns: route.query.ns } : {} })
}
function onKey(e) {
  if ((e.metaKey || e.ctrlKey) && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault()
    jump()
  }
}
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <header class="flex h-[56px] items-center gap-3.5 border-b border-line2 bg-head px-4">
    <!-- left: hamburger + breadcrumb / page name -->
    <div class="flex min-w-0 items-center gap-3">
      <button @click="$emit('open-drawer')" class="grid h-8 w-8 place-items-center rounded-lg text-muted hover:bg-surface2 hover:text-fg md:hidden">
        <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
      </button>
      <nav v-if="breadcrumb" class="flex min-w-0 items-center gap-2 text-sm">
        <template v-for="(b, i) in breadcrumb" :key="i">
          <RouterLink v-if="b.to" :to="b.to" class="shrink-0 text-faint hover:text-muted">{{ b.label }}</RouterLink>
          <span v-else class="truncate font-bold text-fg">{{ b.label }}</span>
          <span v-if="i < breadcrumb.length - 1" class="shrink-0 text-faint opacity-50">›</span>
        </template>
      </nav>
      <h1 v-else-if="headTitle" class="truncate text-base font-bold text-fg">{{ headTitle }}</h1>
      <slot name="title-after" />
    </div>

    <!-- right cluster: identity + status only -->
    <div class="ml-auto flex items-center gap-2.5">
      <!-- compact ⌘K jump -->
      <button @click="jump" v-tip="`Search — ⌘K`"
        class="hidden h-8 items-center gap-1.5 rounded-lg border border-line2 bg-surface px-2.5 text-faint transition-colors hover:border-line hover:text-muted sm:inline-flex">
        <VIcon name="search" :size="15" />
        <kbd class="rounded border border-line2 px-1 font-mono text-[10px] text-cap">⌘K</kbd>
      </button>

      <NamespacePicker />

      <span class="hidden items-center gap-1.5 font-mono text-xs text-faint sm:inline-flex">
        <span class="h-[7px] w-[7px] rounded-full bg-ok"></span><span class="hidden lg:inline">live</span>
      </span>

      <span class="h-[22px] w-px bg-line2"></span>

      <RouterLink :to="{ name: 'events' }" v-tip="`Alerts`"
        class="grid h-8 w-8 place-items-center rounded-lg text-muted transition-colors hover:bg-surface2 hover:text-fg">
        <VIcon name="bell" :size="17" />
      </RouterLink>

      <button @click="ui.toggleTheme()" v-tip="`Toggle theme`"
        class="grid h-8 w-8 place-items-center rounded-lg text-muted transition-colors hover:bg-surface2 hover:text-fg">
        <svg v-if="!ui.light" class="h-[18px] w-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
        <svg v-else class="h-[18px] w-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
      </button>

      <!-- version pill — subtle by default; amber only when a newer release is out -->
      <RouterLink v-if="ver.current" :to="{ name: 'about' }"
        v-tip="ver.isOutdated ? `Update available: ${ver.latestTag} — you have v${ver.current}` : `You're on v${ver.current}`"
        class="hidden items-center gap-1.5 rounded-pill border px-2.5 py-0.5 font-mono text-xs transition-colors sm:inline-flex"
        :class="ver.isOutdated
          ? 'border-warn/40 bg-warn/10 text-warn hover:bg-warn/20'
          : 'border-line text-faint hover:border-line2 hover:text-muted'">
        <span v-if="ver.isOutdated" class="h-1.5 w-1.5 rounded-full bg-warn"></span>
        v{{ ver.current }}
      </RouterLink>

      <AccountMenu />
    </div>
  </header>
</template>
