<script setup>
import { onMounted, onBeforeUnmount } from 'vue'
import { RouterView } from 'vue-router'
import ConfirmDialog from './components/ConfirmDialog.vue'

// The sidebar logo gently cycles hue via the --logo-hue CSS var. The favicon is
// NOT animated — it stays the fixed brand mark from /favicon.svg (linked in
// index.html), so the browser tab keeps a stable, recognizable icon.
let timer
onMounted(() => {
  const root = document.documentElement
  let hue = 170 // brand teal
  root.style.setProperty('--logo-hue', String(hue))
  // Honor reduced-motion: pick one color and stop.
  if (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches) return
  timer = setInterval(() => {
    hue = (hue + 5) % 360 // ~11s for a full cycle
    root.style.setProperty('--logo-hue', hue.toFixed(1))
  }, 150)
})
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <RouterView />
  <ConfirmDialog />
</template>
