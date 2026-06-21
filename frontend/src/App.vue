<script setup>
import { onMounted, onBeforeUnmount } from 'vue'
import { RouterView } from 'vue-router'

// One hue source drives BOTH the sidebar logo (via the --logo-hue CSS var) and the
// animated favicon, so they always show the same color. The logo updates smoothly
// every 150ms; the favicon refreshes less often (cheap + avoids hammering the tab).
let timer
onMounted(() => {
  const root = document.documentElement
  let link = document.querySelector("link[rel~='icon']")
  if (!link) { link = document.createElement('link'); link.rel = 'icon'; document.head.appendChild(link) }
  const c = document.createElement('canvas')
  c.width = c.height = 32
  const ctx = c.getContext('2d')
  let hue = 170 // start near the teal brand color

  const drawFavicon = () => {
    if (document.hidden || !ctx) return
    try {
      ctx.clearRect(0, 0, 32, 32)
      ctx.fillStyle = `hsl(${hue} 70% 55%)`
      if (ctx.roundRect) { ctx.beginPath(); ctx.roundRect(4, 4, 24, 24, 7); ctx.fill() }
      else ctx.fillRect(4, 4, 24, 24)
      link.href = c.toDataURL('image/png')
    } catch { /* ignore */ }
  }

  // honor reduced-motion: pick one color and stop
  if (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    root.style.setProperty('--logo-hue', String(hue)); drawFavicon(); return
  }

  let tick = 0
  root.style.setProperty('--logo-hue', String(hue))
  drawFavicon()
  timer = setInterval(() => {
    hue = (hue + 5) % 360 // ~11s for a full cycle
    root.style.setProperty('--logo-hue', hue.toFixed(1))
    if (tick++ % 4 === 0) drawFavicon() // favicon every ~600ms, same hue
  }, 150)
})
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <RouterView />
</template>
