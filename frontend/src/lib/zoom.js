// Encode/decode the chart zoom window in the URL as a human-readable local
// timestamp range: `YYYY:MM:DD-HH:mm:ss~YYYY:MM:DD-HH:mm:ss` (the two endpoints
// are separated by '~' since the timestamp itself contains '-').

const pad = (n) => String(n).padStart(2, '0')

function fmt(ts) {
  const d = new Date(ts * 1000)
  return `${d.getFullYear()}:${pad(d.getMonth() + 1)}:${pad(d.getDate())}-${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

function parse(s) {
  const m = String(s).match(/^(\d{4}):(\d{2}):(\d{2})-(\d{2}):(\d{2}):(\d{2})$/)
  if (!m) return null
  const [Y, Mo, D, H, Mi, S] = m.slice(1).map(Number)
  return Math.floor(new Date(Y, Mo - 1, D, H, Mi, S).getTime() / 1000)
}

// [minTs, maxTs] → URL string (or undefined to drop the param)
export const encodeZoom = (r) => (r && r.length === 2 ? `${fmt(r[0])}~${fmt(r[1])}` : undefined)

// URL string → [minTs, maxTs] or null
export function decodeZoom(str) {
  if (!str) return null
  const [a, b] = String(str).split('~')
  const min = parse(a), max = parse(b)
  return min && max && max > min ? [min, max] : null
}
