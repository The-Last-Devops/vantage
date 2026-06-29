// Pure host-filter logic for the Systems page — no Vue, no reactive state, so it
// lives here as a self-contained module (and is unit-test-friendly).
//
// The search box is a tiny query language: space-separated tokens AND together.
//   cpu>50 disk<30 status:online kind:docker ns:prod web
// Numeric predicates (cpu/mem/disk + >,<,>=,<=,=), key:value predicates, and bare
// text (matched against node name + hostname, wildcard-aware).

/** Percentage of used/total, rounded; null when not computable. */
export const pct = (u, t) => (u != null && t ? Math.round((u / t) * 100) : null)

/** A host is "down" only after it's been silent for more than OFFLINE_MS. This MUST be
 *  comfortably larger than the agent push interval (default 60s), or a healthy host
 *  grazes the boundary just before each push and flickers down→up. 120s (2× the 60s
 *  push) leaves a full interval of headroom + tolerates one missed push.
 *  (Faster down-detection needs a shorter agent INTERVAL; ideally make this interval-aware.) */
export const OFFLINE_MS = 120000
export const online = (s) => !!s.last_seen && Date.now() - new Date(s.last_seen).getTime() < OFFLINE_MS

/** Parse the query string into a list of predicates. */
export function parseQuery(qs) {
  return (qs || '').trim().split(/\s+/).filter(Boolean).map((tok) => {
    let m = tok.match(/^(cpu|mem|disk)(>=|<=|>|<|=)(\d+(?:\.\d+)?)$/i)
    if (m) return { t: 'num', f: m[1].toLowerCase(), op: m[2], v: +m[3] }
    m = tok.match(/^(status|kind|type|cluster|ns|agent|kernel|name|node|system):(.+)$/i)
    if (m) return { t: 'kv', k: m[1].toLowerCase(), v: m[2].toLowerCase() }
    return { t: 'text', v: tok.toLowerCase() }
  })
}

const metricVal = (s, f) =>
  f === 'cpu' ? s.cpu_percent : f === 'mem' ? pct(s.mem_used, s.mem_total) : pct(s.disk_used, s.disk_total)

const cmpOp = (a, op, b) =>
  op === '>' ? a > b : op === '<' ? a < b : op === '>=' ? a >= b : op === '<=' ? a <= b : a === b

const escapeRe = (s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')

// wildcard match: substring by default; '*' acts as a glob (web*, *-prod, db*1)
function wild(hay, pat) {
  if (!pat) return true
  hay = (hay || '').toLowerCase()
  if (pat.includes('*')) return new RegExp('^' + pat.split('*').map(escapeRe).join('.*') + '$').test(hay)
  return hay.includes(pat)
}

/** Does host `s` satisfy predicate `p`? */
export function matchPred(s, p) {
  if (p.t === 'num') { const v = metricVal(s, p.f); return v != null && cmpOp(v, p.op, p.v) }
  if (p.t === 'kv') {
    if (['name', 'node', 'system'].includes(p.k)) return wild(s.name, p.v)
    if (p.k === 'status') return (online(s) ? 'online' : 'offline').startsWith(p.v)
    if (p.k === 'kind' || p.k === 'type') return s.kind === p.v
    if (p.k === 'cluster') return wild(s.cluster, p.v)
    if (p.k === 'ns') return wild(s.namespace, p.v)
    if (p.k === 'agent') return wild(s.agent_version, p.v)
    if (p.k === 'kernel') return wild(s.kernel, p.v)
  }
  // default (plain text) = node name (+ hostname), wildcard-aware
  return wild(s.name + ' ' + (s.hostname || ''), p.v)
}
