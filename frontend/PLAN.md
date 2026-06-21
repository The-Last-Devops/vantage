# Plan for the real Frontend (Vue SPA) — full version

Goal: port the prototype `ui-prototype/simple` → a real Vue app, consuming the hub's JSON API,
displaying **Metrics/Systems** with real data (`scripts/sim-agents.sh` is available for testing).
Monitors & Status page are **deferred**.

> Open decisions have been locked in with **recommended defaults** (sections 2 & 3) so we don't
> get stuck once we start. To change them, edit there before running milestone 1.

---

## 1. Stack (locked)
- **Vite + Vue 3 + vue-router + Pinia** — already scaffolded in `frontend/`.
- **Tailwind via PostCSS** (no CDN) — teal/navy palette matching the prototype.
- **Charts: uPlot** (per CLAUDE.md) — thin Vue wrapper.
- **Auth**: session cookie, `fetch(credentials:'include')`, same-origin.

## 2. [LOCKED] Hub serves the SPA = **embed `dist/` into the binary** (rust-embed)
Rationale: preserve the project's **single-binary** philosophy.
- Add the `rust-embed` crate; the `frontend` build script runs `vite build` → `frontend/dist`.
- Hub has a fallback handler: a route not matching `/api/*` → serve a static file from `dist`,
  file not found → return `index.html` (SPA history fallback).
- Dev: run `vite` (:5173) proxying `/api`→:8080; no need to rebuild the hub when editing the UI.
- Prod/CI: `vite build` before `cargo build` of the hub (add a step to Dockerfile.hub).

## 3. [LOCKED] Migration = **strangler** (run in parallel, then cut over)
- Early phase: Vue served at **`/app/*`**, the old SSR (`ui.rs`) stays as-is at `/`.
  → don't break what's running; review as we go.
- Once milestones 2–5 are done & stable: **cut `/` over to Vue**, gradually remove the routes + `ui.rs` code.
- Pros: safe, easy rollback; cons: two UIs exist temporarily (acceptable).

---

## 4. API: existing vs needed
**Already enough for most reads:**
- `POST /api/auth/login` · `POST /api/auth/logout` · `GET /api/me`
- `GET /api/servers` — already has `kind` (`node`/`docker`/`k8s`) + `cluster` → group client-side
- `GET /api/servers/:id/metrics` (history) · `/containers` · `/temps` · `/gpu`
- `GET /api/namespaces` · CRUD namespaces/servers/tokens/members
- `DELETE /api/servers/:id` (bulk-delete) · `POST /api/namespaces/:id/tokens` (Add system)

**Needed (phase-tagged):**
- [P-realtime] **SSE live 1s**: `GET /api/servers/:id/stream` (+ a fleet-wide stream for the dashboard).
- [P-realtime] **Write 1m rollups** instead of writing raw on every report (per the `metrics-storage-tiers` memory).
- [M4] `server_metrics` accepts **range + bucket** params (1m/5m/15m/1h) so the chart changes with the range.
- [M4] (optional) a **container history** endpoint for the stacked-area docker view (already have the `/containers` snapshot).

---

## 5. Milestones (in order; only move on once each one runs)

### M1 — Foundation
- `frontend/`: finish `pnpm install` (approve the esbuild build), vite config proxying `/api`.
- Tailwind config + `src/style.css` tokens (ink/panel/panel2/line/teal) — ported from the prototype.
- `src/lib/api.js` (fetch wrapper, credentials), `src/stores/auth.js` (me/login/logout).
- `src/router/index.js` + guard (not logged in → /app/login).
- Hub: add rust-embed + a fallback route serving `dist` at `/app` (section 2).
- **Done when**: visiting `/app` shows an empty shell + redirect to login; the `vite build` embed works through the hub.

### M2 — Login
- `pages/Login.vue` (port the prototype's UI), calling `auth.login`.
- Guard: already logged in → go to Systems; wrong password → show an error.
- **Done when**: logging in with `admin@local/admin123` reaches Systems.

### M3 — Systems dashboard
- `components/AppShell.vue` (sidebar + topbar, theme toggle, multi-select namespace switcher).
- `pages/Systems.vue`: hero (online/avg CPU/mem from `/api/servers`), 3 sections
  **Nodes / Docker / Kubernetes** (grouped by `kind`; k8s grouped by `cluster`).
- Table: column sort, filter, checkbox + **bulk delete** (`DELETE /api/servers/:id`).
- K8s cluster = an aggregate row (client-side aggregation of nodes in the same `cluster`), expandable to nodes.
- Docker = a host row, expandable to containers (`/api/servers/:id/containers`).
- **Done when**: running `sim-agents.sh` → shows exactly 3 sections, figures match, sort/filter/select/delete work.

### M4 — System detail (by `kind`)
- `pages/SystemDetail.vue` route `/app/system/:id` (+ a type query for leaves).
- **node/host**: uPlot charts from `/api/servers/:id/metrics` (CPU/Mem/Disk/IO/Net + Temp/GPU if present), range selector → resolution.
- **docker**: host snapshot (current figures) + a "Host metrics" button + a **Containers** table (click → container detail).
- **k8s cluster**: compact overview + a **Nodes** table (click a node → host charts).
- Hierarchical breadcrumb; empty-state for Temp/GPU when there's no sensor.
- **Done when**: clicking from Systems lands on the right layout per type, charts drawn from real data.

### M5 — Theme (done properly)
- Color tokens via **CSS variables** (`:root` dark, `.light` override) — no hacks like the prototype.
- Toggle in the topbar, persisted to `localStorage`, applied app-wide (including login).
- Clear row hover/select in both modes (defined in the prototype, moved to variables).
- **Done when**: the toggle is smooth and every page changes theme consistently.

### M6 — Add system
- Node/Docker/K8s modal + install-command tabs (Binary/Docker/Compose; Helm/Manifest).
- Create a token implicitly via `POST /api/namespaces/:id/tokens`, embed it in the command, don't expose the token in the UI.
- Customizable namespace dropdown.
- **Done when**: creating a new system → token generated implicitly → install command correct per type.

### M7 — Realtime (backend + frontend phase)
- Backend: write **1m rollups** (aggregate reports in RAM → write one row per minute); `IngestAck.next_interval_secs` so the agent pushes ~1s when a viewer is present.
- Backend: an **SSE** endpoint fanning out live reports to open viewers.
- Frontend: overlay a "Live 1s" point on the chart + update gauges/sparklines in realtime.
- Retention/compression already configured (the `metrics-storage-tiers` memory).
- **Done when**: opening a detail view → live point ticks every 1s; closing the tab → agent reduces its frequency.

### M8 — Cutover + cleanup
- Move `/` to Vue, remove the SSR route in `main.rs` + the unused `ui.rs` code.
- Update CLAUDE.md (frontend = embedded Vue SPA, no more HTMX/SSR for the ported pages).
- **Done when**: a single UI, binary still a single file.

---

## 6. Testing each step
- `bash scripts/sim-agents.sh` (node/docker/k8s fleet, real data) → render & interact.
- `bash scripts/sim-reset.sh` when test data needs clearing.
- `bash scripts/check-systems.sh` to verify the API.
- Change the scale: `NODES=80 DOCKER=15 K8S_CLUSTERS=4 bash scripts/sim-agents.sh`.

## 7. Risks / notes
- **Same-origin cookie**: dev via the Vite proxy; prod on the same hub → OK. If we later split domains (Cloudflare Pages) → switch the cookie to `SameSite=None;Secure` + CORS, or have Pages proxy `/api`.
- **uPlot** lacks nice types → thin wrapper, loaded as a script.
- **Large scope** → strictly work sequentially by milestone; no leapfrogging.
- **Strangler**: keep the SSR until M8 before removing it, to avoid losing features midway.

## 8. Suggested execution order (single pass)
M1 → M2 → M3 → M4 → M5 → M6 → (M7 realtime, large, can be split into a separate batch) → M8 cutover.
Viewable MVP = through **M4** (Login + Systems + Detail with real data).
