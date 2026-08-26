<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { renderMarkdown } from '../lib/markdown'

const REPO = 'The-Last-Devops/vantage'
const about = ref(null)
const latest = ref(null)   // { tag, url, body, published }
const checking = ref(false)
// Expanded automatically when there is something new to read; collapsed when you are
// already current, so the page still leads with the version facts.
const notesOpen = ref(false)
const checkErr = ref('')

const isNewer = computed(() => {
  if (!about.value || !latest.value) return false
  const cur = about.value.version
  const tag = latest.value.tag.replace(/^v/, '')
  return cmp(tag, cur) > 0
})
// We're running a build NEWER than the latest GitHub release (a :main /
// pre-release dev build). Don't claim "you're behind".
const isAhead = computed(() => {
  if (!about.value || !latest.value) return false
  return cmp(about.value.version, latest.value.tag.replace(/^v/, '')) > 0
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
    notesOpen.value = isNewer.value
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
    <div class="space-y-5">
      <div class="flex items-center gap-3">
        <VLogo :size="40" />
        <div>
          <div class="text-lg font-semibold text-fg">Vantage</div>
          <div class="text-sm text-muted">Self-hosted DevOps control plane — monitor, alert &amp; operate servers, clusters, services &amp; cloud</div>
        </div>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface">
        <dl class="divide-y divide-line/60 text-sm">
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Version</dt><dd class="font-mono text-fg">{{ about?.version || '—' }}</dd></div>
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Build</dt><dd class="font-mono text-fg">{{ (about?.git_sha || '—').slice(0, 12) }}</dd></div>
          <div class="flex justify-between px-4 py-3"><dt class="text-faint">Built</dt><dd class="font-mono text-fg">{{ about?.build_date || '—' }}</dd></div>
        </dl>
      </div>

      <!-- update check -->
      <div class="rounded-xl border p-4" :class="isNewer ? 'border-warn/40 bg-warn/5' : 'border-line bg-surface'">
        <div class="flex items-center justify-between gap-3">
          <div>
            <div v-if="checking" class="text-sm text-muted">Checking for updates…</div>
            <div v-else-if="checkErr" class="text-sm text-muted">{{ checkErr }}</div>
            <div v-else-if="isNewer" class="text-sm font-medium text-warn">Update available: {{ latest.tag }} (you have v{{ about?.version }})</div>
            <div v-else-if="isAhead" class="text-sm font-medium text-accent">Running a pre-release (v{{ about?.version }}) — ahead of the latest release {{ latest.tag }}</div>
            <div v-else-if="latest" class="text-sm font-medium text-accent">You're on the latest version (v{{ about?.version }})</div>
          </div>
          <a :href="`https://github.com/${REPO}/releases`" target="_blank" rel="noopener" class="shrink-0 rounded-lg border border-line bg-surface2 px-3 py-1.5 text-sm text-fg hover:border-accent/50">Releases ↗</a>
        </div>
        <!-- Release notes: shown whenever we have them, not only when behind — "what
             changed in the version I am running" is the common question, and the notes
             used to vanish the moment you were up to date. renderMarkdown escapes the
             whole body before adding tags (it comes from api.github.com). -->
        <div v-if="latest?.body" class="mt-3">
          <button @click="notesOpen = !notesOpen" class="flex items-center gap-1.5 text-xs font-semibold text-muted hover:text-fg">
            <svg class="h-3.5 w-3.5 transition-transform" :class="notesOpen ? 'rotate-90' : ''" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="m9 18 6-6-6-6"/></svg>
            {{ isNewer ? `What's new in ${latest.tag}` : `Release notes — ${latest.tag}` }}
            <span v-if="latest.published" class="font-normal text-faint">· {{ new Date(latest.published).toLocaleDateString() }}</span>
          </button>
          <div v-if="notesOpen" class="mt-2 max-h-[60vh] overflow-auto rounded-lg bg-bg p-3.5 text-xs text-muted" v-html="renderMarkdown(latest.body)"></div>
        </div>
      </div>

      <p class="text-xs text-faint">Changelog &amp; source: <a :href="`https://github.com/${REPO}`" target="_blank" rel="noopener" class="text-accent hover:underline">github.com/{{ REPO }}</a></p>
    </div>
  </AppShell>
</template>
