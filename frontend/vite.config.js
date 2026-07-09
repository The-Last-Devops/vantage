import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// SPA served under /app (embedded in the hub binary for prod). Dev proxies /api
// to the hub on :8080 so the session cookie stays same-origin.
export default defineConfig({
  base: '/',
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      '/api': { target: 'http://localhost:8080', changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      // Split the uPlot charting vendor into its own chunk so it caches independently
      // of app code. Routes stay eagerly imported (no lazy chunks — see router header).
      output: { manualChunks: { uplot: ['uplot'] } },
    },
  },
})
