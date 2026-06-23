// Thin fetch wrapper. All requests are same-origin (dev: Vite proxies /api → hub).
// `credentials: 'include'` carries the httpOnly session cookie.
async function request(method, path, body) {
  const opts = {
    method,
    credentials: 'include',
    cache: 'no-store', // polled endpoints must never come from the HTTP cache (was causing delayed "realtime" updates)
    headers: {},
  }
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  const res = await fetch(path, opts)
  if (!res.ok) {
    const err = new Error(`${method} ${path} → ${res.status}`)
    err.status = res.status
    throw err
  }
  if (res.status === 204) return null
  const ct = res.headers.get('content-type') || ''
  return ct.includes('application/json') ? res.json() : res.text()
}

export const api = {
  get: (p) => request('GET', p),
  post: (p, b) => request('POST', p, b),
  put: (p, b) => request('PUT', p, b),
  patch: (p, b) => request('PATCH', p, b),
  del: (p) => request('DELETE', p),
}
