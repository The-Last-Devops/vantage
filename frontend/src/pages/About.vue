<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'

const REPO = 'The-Last-Devops/last-monitor'
const about = ref(null)
const latest = ref(null)   // { tag, url, body, published }
const checking = ref(false)
const checkErr = ref('')

const isNewer = computed(() => {
  if (!about.value || !latest.value) return false
  const cur = about.value.version
  const tag = latest.value.tag.replace(/^v/, '')
  return tag !== cur && cmp(tag, cur) > 0
})
// naive semver compare
function cmp(a, b) {
  const pa = a.split('.').map(Number), pb = b.split('.').map(Number)
  for (let i = 0; i < 3; i++) { if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0) }
  return 0
}

async function checkLatest() {
  checking.value = true; checkErr.value = ''
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } })
    if (!r.ok) throw new Error(r.status)
    const j = await r.json()
    latest.value = { tag: j.tag_name, url: j.html_url, body: j.body || '', published: j.published_at }
  } catch (e) { checkErr.value = 'Could not reach GitHub.' }
  finally { checking.value = false }
}

onMounted(async () => {
  try { about.value = await api.get('/api/about') } catch {}
  checkLatest()
})
</script>

<template>
  <AppShell title="About">
    <div class="max-w-2xl space-y-5">
      <div class="flex items-center gap-3">
        <span class="lm-logo inline-block h-10 w-10 rounded-lg"></span>
        <div>
          <div class="text-lg font-semibold text-fg">Last Monitor</div>
          <div class="text-sm text-muted">Self-hosted infrastructure &amp; service monitoring</div>
        </div>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <dl class="divide-y divide-line/60 text-sm">
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Version</dt><dd class="font-mono text-fg">{{ about?.version || '—' }}</dd></div>
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Build</dt><dd class="font-mono text-fg">{{ about?.git_sha || '—' }}</dd></div>
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Built</dt><dd class="font-mono text-fg">{{ about?.build_date || '—' }}</dd></div>
        </dl>
      </div>

      <!-- update check -->
      <div class="rounded-xl border p-4" :class="isNewer ? 'border-amber-500/40 bg-amber-500/5' : 'border-line bg-surface'">
        <div class="flex items-center justify-between gap-3">
          <div>
            <div v-if="checking" class="text-sm text-muted">Checking for updates…</div>
            <div v-else-if="checkErr" class="text-sm text-muted">{{ checkErr }}</div>
            <div v-else-if="isNewer" class="text-sm font-medium text-amber-400">Update available: {{ latest.tag }} (you have {{ about?.version }})</div>
            <div v-else-if="latest" class="text-sm font-medium text-accent">You're on the latest version ({{ latest.tag }})</div>
          </div>
          <a :href="`https://github.com/${REPO}/releases`" target="_blank" rel="noopener" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg hover:border-accent/50">Releases ↗</a>
        </div>
        <pre v-if="latest?.body && isNewer" class="mt-3 max-h-48 overflow-auto whitespace-pre-wrap rounded-lg bg-bg p-3 text-xs leading-relaxed text-muted">{{ latest.body }}</pre>
      </div>

      <p class="text-xs text-faint">Changelog &amp; source: <a :href="`https://github.com/${REPO}`" target="_blank" rel="noopener" class="text-accent hover:underline">github.com/{{ REPO }}</a></p>
    </div>
  </AppShell>
</template>
