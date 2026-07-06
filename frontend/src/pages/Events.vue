<script setup>
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import AppShell from '../components/AppShell.vue'
import PageLoader from '../components/PageLoader.vue'
import { api } from '../lib/api'
import { useCached } from '../lib/cache'

const route = useRoute()
const wsQuery = computed(() => (route.query.ws ? { ws: route.query.ws } : {}))

const workspaces = ref([])
// Workspaces in scope: the sidebar selection (?ws), or all when none chosen.
const selectedWsNames = computed(() => (route.query.ws || '').split(',').filter(Boolean))
const activeWs = computed(() =>
  selectedWsNames.value.length
    ? workspaces.value.filter((n) => selectedWsNames.value.includes(n.name))
    : workspaces.value,
)
const alerts = ref([])
const events = ref([])
const filterStatus = ref('all') // all | active | resolved
const filterSource = ref('all') // all | monitor | host
let timer = null

const { loaded, reload: load } = useCached({
  key: () => 'events:' + activeWs.value.map((n) => n.id).join(','),
  load: async () => {
    const nss = activeWs.value
    if (!nss.length) return { alerts: [], events: [] }
    const [aLists, eLists] = await Promise.all([
      Promise.all(nss.map((n) =>
        api.get(`/api/workspaces/${n.id}/alerts`)
          .then((rows) => rows.map((r) => ({ ...r, workspace: n.name })))
          .catch(() => []),
      )),
      Promise.all(nss.map((n) =>
        api.get(`/api/workspaces/${n.id}/alert-events`)
          .then((rows) => rows.map((r) => ({ ...r, workspace: n.name })))
          .catch(() => []),
      )),
    ])
    return {
      alerts: aLists.flat(),
      // one global feed, newest first across all selected workspaces
      events: eLists.flat().sort((x, y) => new Date(y.at) - new Date(x.at)),
    }
  },
  apply: (d) => { alerts.value = d.alerts; events.value = d.events },
  onError: () => { alerts.value = []; events.value = [] },
})
watch(() => route.query.ws, load)

// active = enabled rules currently firing
const active = computed(() =>
  alerts.value
    .filter((a) => a.firing === true && a.enabled)
    .filter((a) => filterSource.value === 'all' || a.target_kind === filterSource.value),
)

// History rows, annotated with a duration for recoveries (paired with the most
// recent prior "firing" event of the same rule).
const history = computed(() => {
  const rows = events.value.filter(
    (e) => filterSource.value === 'all' || e.target_kind === filterSource.value,
  )
  return rows.map((e, i) => {
    let durMs = null
    if (!e.firing) {
      for (let j = i + 1; j < rows.length; j++) {
        if (rows[j].alert_id === e.alert_id && rows[j].firing) {
          durMs = new Date(e.at).getTime() - new Date(rows[j].at).getTime()
          break
        }
      }
    }
    return { ...e, durMs }
  })
})
const shownHistory = computed(() =>
  history.value.filter((e) =>
    filterStatus.value === 'all' ? true : filterStatus.value === 'active' ? e.firing : !e.firing,
  ),
)

// paginate the (time-growing) history so it can't run off the page
const HIST_PAGE = 30
const histPage = ref(1)
const histPages = computed(() => Math.max(1, Math.ceil(shownHistory.value.length / HIST_PAGE)))
const pagedHistory = computed(() => shownHistory.value.slice((histPage.value - 1) * HIST_PAGE, histPage.value * HIST_PAGE))
watch(shownHistory, () => { if (histPage.value > histPages.value) histPage.value = 1 })

function ruleLink(id) { return { name: 'alerts', query: { ...wsQuery.value, rule: id } } }
function dur(ms) {
  if (ms == null) return ''
  let s = Math.max(0, ms / 1000)
  const h = Math.floor(s / 3600); s -= h * 3600
  const m = Math.floor(s / 60); s -= m * 60
  if (h) return `${h}h ${m}m`
  if (m) return `${m}m ${Math.floor(s)}s`
  return `${Math.floor(s)}s`
}
function since(iso) {
  let s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000)
  const d = Math.floor(s / 86400); s -= d * 86400
  const h = Math.floor(s / 3600); s -= h * 3600
  const m = Math.floor(s / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  return `${m}m`
}
const evTime = (iso) => new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false })

onMounted(async () => {
  try { workspaces.value = await api.get('/api/workspaces') } catch {}
  await load()
  timer = setInterval(load, 15000)
})
onUnmounted(() => clearInterval(timer))
</script>

<template>
  <AppShell title="Events">
    <div class="space-y-5">
      <p class="text-sm text-muted">A live record of what actually happened — incidents firing now and the resolved history. Populated automatically when an <RouterLink :to="{ name: 'alerts', query: wsQuery }" class="text-accent hover:underline">alert rule</RouterLink> trips.</p>

      <!-- filters -->
      <div class="flex flex-wrap gap-2">
        <div class="inline-flex overflow-hidden rounded-lg border border-line">
          <button v-for="f in ['all', 'active', 'resolved']" :key="f" @click="filterStatus = f"
            class="px-3 py-1.5 text-xs capitalize" :class="filterStatus === f ? 'bg-surface2 text-fg' : 'text-muted hover:text-fg'">{{ f }}</button>
        </div>
        <div class="inline-flex overflow-hidden rounded-lg border border-line">
          <button v-for="[v, l] in [['all', 'All sources'], ['monitor', 'Services'], ['host', 'Hosts']]" :key="v" @click="filterSource = v"
            class="px-3 py-1.5 text-xs" :class="filterSource === v ? 'bg-surface2 text-fg' : 'text-muted hover:text-fg'">{{ l }}</button>
        </div>
      </div>

      <PageLoader v-if="!loaded" />

      <!-- empty -->
      <div v-else-if="!active.length && !shownHistory.length" class="flex flex-col items-center gap-3.5 rounded-2xl border border-line bg-surface/50 px-7 py-12 text-center">
        <span class="grid h-16 w-16 place-items-center rounded-2xl border border-accent/30 bg-accent/10 text-accent">
          <svg class="h-7 w-7" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>
        </span>
        <h2 class="text-base font-semibold text-fg">All clear — no events</h2>
        <p class="max-w-md text-sm text-muted">Nothing has tripped a rule yet. When a host or service breaches a rule, the incident shows up here with its timeline.</p>
        <RouterLink :to="{ name: 'alerts', query: wsQuery }" class="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface2 px-3.5 py-2 text-sm font-semibold text-fg hover:border-accent/50">Review alert rules</RouterLink>
      </div>

      <template v-else>
        <!-- active now -->
        <section v-if="active.length" class="space-y-2.5">
          <div class="flex items-center gap-2.5 text-[11px] font-bold uppercase tracking-wider text-faint">
            <span class="relative h-2.5 w-2.5 rounded-full bg-down"><span class="absolute inset-0 animate-ping rounded-full bg-down/60"></span></span>
            Active now <span class="rounded-full bg-surface2 px-2 py-0.5 text-[10px]">{{ active.length }}</span>
          </div>
          <RouterLink v-for="a in active" :key="a.id" :to="ruleLink(a.id)"
            class="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-down/40 bg-down/5 px-4 py-2.5 transition-colors hover:border-down/70">
            <span class="h-2.5 w-2.5 shrink-0 rounded-full bg-down"></span>
            <span class="text-sm font-semibold text-fg">{{ a.target_name || '—' }} is DOWN</span>
            <span class="rounded-md bg-down/15 px-1.5 py-0.5 font-mono text-[11px] font-bold text-down">{{ since(a.since) }}</span>
            <span class="text-[11px] uppercase text-faint">{{ a.target_kind }}</span>
            <span class="inline-flex items-center gap-1 text-xs text-faint"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/></svg>{{ a.workspace }}</span>
            <span v-if="a.since" class="text-xs text-faint">since {{ evTime(a.since) }}</span>
            <span v-if="a.channels?.length" class="truncate text-xs text-faint">via {{ a.channels.map((c) => c.name).join(', ') }}</span>
            <span class="flex-1"></span>
            <svg class="h-4 w-4 shrink-0 text-faint" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m9 18 6-6-6-6"/></svg>
          </RouterLink>
        </section>

        <!-- earlier -->
        <section v-if="shownHistory.length" class="space-y-1">
          <div class="text-[11px] font-bold uppercase tracking-wider text-faint">Earlier</div>
          <div class="max-h-[70vh] overflow-y-auto rounded-xl border border-line bg-surface">
            <RouterLink v-for="(e, i) in pagedHistory" :key="i" :to="ruleLink(e.alert_id)"
              class="flex items-start gap-3.5 border-b border-line/60 px-4 py-3 transition-colors last:border-0 hover:bg-surface2">
              <span class="mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-lg"
                :class="e.firing ? 'bg-down/15 text-down' : 'bg-accent/15 text-accent'">
                <svg v-if="e.firing" class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m10.29 3.86-8.48 14.7A2 2 0 0 0 3.53 21h16.94a2 2 0 0 0 1.72-2.44L13.71 3.86a2 2 0 0 0-3.42 0Z"/><path d="M12 9v4M12 17h.01"/></svg>
                <svg v-else class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5"/></svg>
              </span>
              <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2.5 text-[13.5px] font-medium text-fg">
                  {{ e.target_name || '—' }} {{ e.firing ? 'went DOWN' : 'recovered' }}
                  <span v-if="e.durMs != null" class="rounded-md bg-accent/12 px-1.5 py-0.5 font-mono text-[11px] text-accent">was down {{ dur(e.durMs) }}</span>
                </div>
                <div class="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-xs text-faint">
                  <span class="inline-flex items-center gap-1"><svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/></svg>{{ e.workspace }}</span>
                  <span class="truncate">{{ e.message || (e.target_kind === 'monitor' ? 'Service check' : 'Host condition') }}</span>
                </div>
              </div>
              <span class="shrink-0 pl-2 text-xs font-mono tabular-nums text-faint">{{ evTime(e.at) }}</span>
            </RouterLink>
          </div>
          <div v-if="histPages > 1" class="flex items-center justify-between px-1 pt-1 text-xs">
            <button :disabled="histPage <= 1" @click="histPage--"
              class="rounded-lg border border-line px-2.5 py-1 text-muted hover:border-accent/50 hover:text-fg disabled:opacity-40">Prev</button>
            <span class="font-mono tabular-nums text-faint">Page {{ histPage }} / {{ histPages }} · {{ shownHistory.length }} events</span>
            <button :disabled="histPage >= histPages" @click="histPage++"
              class="rounded-lg border border-line px-2.5 py-1 text-muted hover:border-accent/50 hover:text-fg disabled:opacity-40">Next</button>
          </div>
        </section>
      </template>
    </div>
  </AppShell>
</template>
