<script setup>
import { ref, computed, onMounted } from 'vue'
import AppShell from '../components/AppShell.vue'
import { api } from '../lib/api'
import { useAuth } from '../stores/auth'

const auth = useAuth()
const isAdmin = computed(() => !!auth.user?.is_admin)

const includeMetrics = ref(false)
const restoreFile = ref(null)
const msg = ref('')
const busy = ref(false)

// S3
const s3 = ref({ endpoint: '', region: 'us-east-1', bucket: '', access_key: '', secret_key: '', prefix: '' })
const s3State = ref({ configured: false, secret_set: false })
const s3keys = ref([])
const s3msg = ref('')

const downloadHref = computed(() => `/api/admin/backup?metrics=${includeMetrics.value}`)

function flash(target, text, ok = true) {
  const r = target === 's3' ? s3msg : msg
  r.value = (ok ? '✓ ' : '') + text
}

async function uploadRestore() {
  if (!restoreFile.value) { flash('local', 'Pick a backup file first.', false); return }
  if (!confirm('Restore will OVERWRITE all current configuration with the backup. Continue?')) return
  busy.value = true; msg.value = ''
  try {
    const res = await fetch('/api/admin/restore', { method: 'POST', credentials: 'include', body: restoreFile.value })
    if (!res.ok) throw new Error(await res.text() || res.status)
    flash('local', 'Restore complete. You may need to log in again.')
  } catch (e) { flash('local', `Restore failed: ${e.message}`, false) }
  finally { busy.value = false }
}

async function loadS3() {
  try {
    const r = await api.get('/api/admin/backup/s3')
    s3State.value = r
    if (r.configured) s3.value = { endpoint: r.endpoint, region: r.region, bucket: r.bucket, access_key: r.access_key, secret_key: '', prefix: r.prefix || '' }
  } catch {}
}
async function saveS3() {
  s3msg.value = ''
  try { await api.put('/api/admin/backup/s3', s3.value); flash('s3', 'Saved.'); await loadS3(); await listS3() }
  catch (e) { flash('s3', `Save failed (${e.status}).`, false) }
}
async function testS3() {
  s3msg.value = ''
  try { await api.post('/api/admin/backup/s3/test'); flash('s3', 'Connection OK.') }
  catch (e) { flash('s3', `Test failed (${e.status}).`, false) }
}
async function uploadS3() {
  busy.value = true; s3msg.value = ''
  try { const r = await api.post(`/api/admin/backup/s3/upload?metrics=${includeMetrics.value}`); flash('s3', `Uploaded ${r.key}.`); await listS3() }
  catch (e) { flash('s3', `Upload failed (${e.status}).`, false) }
  finally { busy.value = false }
}
async function listS3() {
  try { const r = await api.get('/api/admin/backup/s3/list'); s3keys.value = r.keys || [] } catch { s3keys.value = [] }
}
async function restoreS3(key) {
  if (!confirm(`Restore from ${key}? This OVERWRITES all current configuration.`)) return
  busy.value = true; s3msg.value = ''
  try { await api.post('/api/admin/backup/s3/restore', { key }); flash('s3', 'Restore complete. You may need to log in again.') }
  catch (e) { flash('s3', `Restore failed (${e.status}).`, false) }
  finally { busy.value = false }
}

onMounted(() => { if (isAdmin.value) { loadS3(); listS3() } })
</script>

<template>
  <AppShell title="Backup &amp; restore">
    <div v-if="!isAdmin" class="mx-auto max-w-md rounded-xl border border-line bg-surface p-6 text-center text-muted">
      Only system admins can manage backups.
    </div>
    <div v-else class="max-w-3xl space-y-6">
      <p class="text-sm text-muted">A backup is a snapshot of the <b>configuration</b> (users, namespaces, RBAC, API keys, servers, monitors, alerts, channels, status pages). Optionally include the <b>metrics</b> history (much larger). Restore <b>overwrites</b> everything — admin only.</p>

      <label class="flex items-center gap-2 text-sm text-fg">
        <input v-model="includeMetrics" type="checkbox" class="h-4 w-4" />
        Include metrics history (system / container metrics + heartbeats) — larger file
      </label>

      <!-- download / upload -->
      <section class="rounded-xl border border-line bg-surface p-4 space-y-3">
        <h2 class="text-sm font-semibold text-fg">Download / upload</h2>
        <div class="flex flex-wrap items-center gap-3">
          <a :href="downloadHref" class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">Download backup</a>
          <span class="text-faint">·</span>
          <input type="file" accept=".gz,.json,application/gzip,application/json" @change="restoreFile = $event.target.files[0]" class="text-sm text-muted file:mr-3 file:rounded-lg file:border file:border-line file:bg-surface2 file:px-3 file:py-1.5 file:text-sm file:text-fg" />
          <button :disabled="busy" @click="uploadRestore" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50 disabled:opacity-50">Restore from file</button>
        </div>
        <p v-if="msg" class="text-xs" :class="msg.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ msg }}</p>
      </section>

      <!-- S3 -->
      <section class="rounded-xl border border-line bg-surface p-4 space-y-3">
        <div class="flex items-center gap-2">
          <h2 class="text-sm font-semibold text-fg">S3 / S3-compatible</h2>
          <span v-if="s3State.configured" class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-accent">configured</span>
        </div>
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <label class="text-xs text-faint">Endpoint<input v-model="s3.endpoint" placeholder="https://s3.us-east-1.amazonaws.com" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Bucket<input v-model="s3.bucket" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Region<input v-model="s3.region" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Key prefix (optional)<input v-model="s3.prefix" placeholder="last-monitor/" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg placeholder:text-faint focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Access key<input v-model="s3.access_key" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
          <label class="text-xs text-faint">Secret key <span v-if="s3State.secret_set" class="text-faint">(leave blank to keep)</span><input v-model="s3.secret_key" type="password" class="mt-1 block w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg focus:border-accent/60 focus:outline-none" /></label>
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <button @click="saveS3" class="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accentfg hover:opacity-90">Save</button>
          <button @click="testS3" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50">Test connection</button>
          <button :disabled="busy" @click="uploadS3" class="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-fg hover:border-accent/50 disabled:opacity-50">Back up to S3 now</button>
          <span v-if="s3msg" class="text-xs" :class="s3msg.startsWith('✓') ? 'text-accent' : 'text-rose-400'">{{ s3msg }}</span>
        </div>

        <div v-if="s3keys.length" class="mt-2 overflow-hidden rounded-lg border border-line">
          <table class="w-full text-sm">
            <thead><tr class="border-b border-line text-left text-[11px] uppercase tracking-wider text-faint"><th class="px-3 py-2 font-medium">Backup object</th><th class="px-3 py-2"></th></tr></thead>
            <tbody>
              <tr v-for="k in s3keys" :key="k" class="border-b border-line/60 last:border-0 hover:bg-surface2/40">
                <td class="px-3 py-2 font-mono text-xs text-muted">{{ k }}</td>
                <td class="px-3 py-2 text-right"><button :disabled="busy" @click="restoreS3(k)" class="rounded-md border border-line bg-surface2 px-2.5 py-1 text-xs text-fg hover:border-accent/50 disabled:opacity-50">Restore</button></td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </AppShell>
</template>
