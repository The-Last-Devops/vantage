<script setup>
import { ref, watch } from 'vue'
import SidebarNav from './SidebarNav.vue'
import AppHeader from './AppHeader.vue'

const props = defineProps({
  title: { type: String, default: '' },
  hideTitle: { type: Boolean, default: false },
  // Breadcrumb shown in the header bar instead of `title` (saves a row vs an
  // in-page breadcrumb). Array of { label, to? }; the last item is the current page.
  breadcrumb: { type: Array, default: null },
})

const drawer = ref(false)

// browser tab title follows the page (e.g. "docker-01 — Vantage")
watch(() => props.title, (t) => { document.title = t ? `${t} — Vantage` : 'Vantage' }, { immediate: true })
</script>

<template>
  <div class="flex min-h-screen">
    <div v-if="drawer" class="fixed inset-0 z-30 bg-black/60 md:hidden" @click="drawer = false"></div>

    <!-- sidebar -->
    <SidebarNav :drawer="drawer" />

    <!-- main -->
    <div class="flex min-w-0 flex-1 flex-col">
      <AppHeader :title="title" :hide-title="hideTitle" :breadcrumb="breadcrumb" @open-drawer="drawer = true">
        <template #title-after><slot name="title-after" /></template>
        <template #actions><slot name="actions" /></template>
        <template #header><slot name="header" /></template>
      </AppHeader>
      <main class="flex-1 p-4 sm:p-6"><slot /></main>
    </div>
  </div>
</template>

<style scoped>
.vantage-soon { display: flex; align-items: center; gap: 0.625rem; border-radius: 0.5rem; padding: 0.5rem 0.75rem; font-size: 0.875rem; color: rgb(var(--faint)); cursor: not-allowed; }
.vantage-soon-pill { margin-left: auto; border-radius: 0.25rem; background: rgb(var(--surface2)); padding: 0.05rem 0.4rem; font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em; }
</style>
