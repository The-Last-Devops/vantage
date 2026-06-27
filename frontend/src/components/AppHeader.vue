<script setup>
import { onMounted } from 'vue'
import { RouterLink } from 'vue-router'
import { useUi } from '../stores/ui'
import { useVersion } from '../stores/version'

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
onMounted(() => ver.ensureLoaded())
</script>

<template>
  <header class="flex min-h-[60px] items-center justify-between gap-3 border-b border-line bg-surface/60 px-4 py-2.5 backdrop-blur sm:px-6">
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
    <div class="flex items-center gap-3">
      <slot name="actions" />
      <slot name="header" />
      <button @click="ui.toggleTheme()" v-tip="`Toggle theme`" class="rounded-lg border border-line bg-surface2 p-1.5 text-muted hover:text-accent">
      <svg v-if="!ui.light" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>
      <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>
      </button>
      <!-- build version → About; green = up to date / unknown, amber = newer release out -->
      <RouterLink v-if="ver.current" :to="{ name: 'about' }"
        v-tip="ver.isOutdated ? `Update available: ${ver.latestTag} — you have v${ver.current}` : `You're on v${ver.current}`"
        class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-colors"
        :class="ver.isOutdated
          ? 'border-amber-400/40 bg-amber-400/10 text-amber-400 hover:bg-amber-400/20'
          : 'border-emerald-400/40 bg-emerald-400/10 text-emerald-400 hover:bg-emerald-400/20'">
        <span class="h-1.5 w-1.5 rounded-full" :class="ver.isOutdated ? 'bg-amber-400' : 'bg-emerald-400'"></span>
        v{{ ver.current }}
      </RouterLink>
    </div>
  </header>
</template>
