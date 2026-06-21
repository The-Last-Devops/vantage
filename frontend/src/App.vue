<script setup>
import { onMounted, onBeforeUnmount } from 'vue'
import { RouterView } from 'vue-router'

// Animated favicon: a rounded square that drifts through the hue spectrum at a
// calm, resting-heartbeat pace. Kept gentle (infrequent, paused when the tab is
// hidden) so it never hammers the browser.
let timer
onMounted(() => {
  let link = document.querySelector("link[rel~='icon']")
  if (!link) { link = document.createElement('link'); link.rel = 'icon'; document.head.appendChild(link) }
  const c = document.createElement('canvas')
  c.width = c.height = 32
  const ctx = c.getContext('2d')
  let hue = 170 // start near the teal brand color
  const draw = () => {
    if (document.hidden || !ctx) return
    try {
      hue = (hue + 8) % 360
      ctx.clearRect(0, 0, 32, 32)
      ctx.fillStyle = `hsl(${hue} 70% 55%)`
      if (ctx.roundRect) { ctx.beginPath(); ctx.roundRect(4, 4, 24, 24, 7); ctx.fill() }
      else ctx.fillRect(4, 4, 24, 24)
      link.href = c.toDataURL('image/png')
    } catch { /* ignore */ }
  }
  draw()
  timer = setInterval(draw, 1500)
})
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <RouterView />
</template>
