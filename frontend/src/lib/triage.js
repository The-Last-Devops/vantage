// Shared host-triage helpers for the Overview + FleetOverview pages — mirrors the
// severity logic already in Systems.vue so the new attention-first pages agree
// with the Systems table on what counts as "warn" / "down". No Vue, no state.
import { pct, online } from './hostFilter'

export const DEFAULT_THR = {
  cpu_warn: 80, cpu_crit: 90, mem_warn: 80, mem_crit: 90,
  disk_warn: 80, disk_crit: 90, dutil_warn: 80, dutil_crit: 95,
}

const metricsOf = (s) => ({
  cpu: s.cpu_percent,
  mem: pct(s.mem_used, s.mem_total),
  disk: pct(s.disk_used, s.disk_total),
  dutil: s.disk_util,
})

const LABEL = { cpu: 'CPU', mem: 'Memory', disk: 'Disk', dutil: 'Disk I/O' }
const ORDER = ['cpu', 'disk', 'mem', 'dutil']

// State for the heatmap/KPIs: 'down' | 'crit' | 'warn' | 'ok'.
//   down = unreachable (no recent heartbeat) — the host is gone.
//   crit = reachable but a metric is past its critical threshold (e.g. disk 93%).
//          NOT "down" — the host is alive, it just needs attention urgently.
//   warn = reachable, a metric is past its warning threshold.
export function hostState(s, thr = DEFAULT_THR) {
  if (!online(s)) return 'down'
  const m = metricsOf(s)
  let warn = false
  for (const k of ORDER) {
    const v = m[k], w = thr[k + '_warn'], c = thr[k + '_crit']
    if (v == null) continue
    if (v >= c) return 'crit'
    if (v >= w) warn = true
  }
  return warn ? 'warn' : 'ok'
}

// Human label + StatePill tone for a host state.
export const STATE_LABEL = { ok: 'Up', warn: 'Warn', crit: 'Critical', down: 'Down' }
// Sort rank, worst first (down and crit both demand attention; down sorts first).
export const STATE_RANK = { down: 0, crit: 1, warn: 2, ok: 3 }

// The single worst metric breach as human text, e.g. "CPU 91%". null if none.
export function worstReason(s, thr = DEFAULT_THR) {
  if (!online(s)) return 'offline'
  const m = metricsOf(s)
  let best = null
  for (const k of ORDER) {
    const v = m[k], w = thr[k + '_warn']
    if (v == null || v < w) continue
    if (!best || v > best.v) best = { k, v }
  }
  return best ? `${LABEL[best.k]} ${Math.round(best.v)}%` : null
}

// "12m" / "3h 04m" / "2d 5h" since an ISO/Date timestamp; '' when unknown.
export function ago(ts) {
  if (!ts) return ''
  const s = Math.max(0, Math.floor((Date.now() - new Date(ts).getTime()) / 1000))
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${String(m).padStart(2, '0')}m`
  if (m) return `${m}m`
  return `${s}s`
}

export { pct, online }
