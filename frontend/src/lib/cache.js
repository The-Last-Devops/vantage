// Stale-while-revalidate cache for page data, so navigating back to a page you've
// already visited paints instantly from memory and refetches in the background —
// instead of remounting blank and re-showing the loader on every navigation.
//
// The store is a module-level reactive Map that survives route changes (unlike a
// component's own refs, which reset on unmount). A page describes its data with
// `useCached({ key, load, apply })`:
//   - key()   → a string identifying this resource for the current inputs
//               (e.g. selected workspaces). A new key fetches fresh; a known key
//               paints from cache first.
//   - load()  → async, returns the data object (does the fetch/merge).
//   - apply() → assigns that data into the page's own refs.
// The composable owns the `loaded` flag, the first-load `minLoad`, and the
// background revalidation timing — so pages stop hand-rolling that plumbing.
import { reactive, ref, onScopeDispose } from 'vue'
import { minLoad } from './minLoad'

const store = reactive(new Map()) // key -> { data, ts } (ts = last fetch time)

// Every currently-mounted cached resource registers its reload() here so we can
// revalidate them all when the tab regains focus / the network reconnects — the
// fix for the classic stale-cache trap (you leave a page open, the data goes out
// of date, and nothing refetches it). This is what SWR/Query libraries do by
// default. Pages that already poll get this for free; the win is the no-poll
// pages (Members, Workspaces, API tokens, retention).
const active = new Set()
let wired = false
function wireGlobalRevalidation() {
  if (wired || typeof window === 'undefined') return
  wired = true
  const kick = () => { if (document.visibilityState !== 'hidden') active.forEach((fn) => fn()) }
  window.addEventListener('focus', kick)
  window.addEventListener('online', kick)
  document.addEventListener('visibilitychange', kick)
}

// Drop cached entries (all, or those whose key starts with `prefix`). Call on
// logout so one user's data never paints for the next; call after a mutation if
// you want a sibling page to refetch rather than show its stale snapshot.
export function dropCache(prefix) {
  for (const k of [...store.keys()]) if (!prefix || k.startsWith(prefix)) store.delete(k)
}

// How old (ms) a cached entry may be before reload() refuses to paint it as
// "ready" and shows the loader instead — a hard ceiling on visible staleness for
// the rare case a page is remounted with a very old snapshot and the network is
// slow to revalidate. Background revalidation still always runs regardless.
const MAX_AGE_MS = 60_000

export function useCached({ key, load, apply, onError, minMs = 200, maxAgeMs = MAX_AGE_MS }) {
  const loaded = ref(false)
  let shownKey = null

  wireGlobalRevalidation()
  active.add(reload)
  onScopeDispose(() => active.delete(reload))

  async function reload() {
    const k = key()
    const hit = store.get(k)
    // 1) Instant paint: if we've seen this key AND it isn't ancient, show it now
    //    (no spinner). A genuinely-unseen key, or one older than maxAgeMs, drops
    //    back to the loader so we never present badly-stale data as current.
    const fresh = hit && Date.now() - hit.ts < maxAgeMs
    if (fresh) {
      apply(hit.data)
      shownKey = k
      loaded.value = true
    } else if (k !== shownKey) {
      loaded.value = false
    }
    // 2) Revalidate in the background. `first` is true only when nothing is shown
    //    yet — that's the one time we gate on minLoad so the spinner reads as
    //    intentional. Polls and cache-hit revalidations stay silent.
    const first = !loaded.value
    const work = (async () => {
      const data = await load()
      store.set(k, { data, ts: Date.now() })
      // Guard against a fast key switch (e.g. workspace change) landing an older
      // response after the user already moved on.
      if (key() === k) { apply(data); shownKey = k; loaded.value = true }
      return data
    })()
    try {
      await (first ? minLoad(work, minMs) : work)
    } catch (e) {
      if (key() === k) { onError?.(e); loaded.value = true }
    }
  }

  return { loaded, reload }
}
