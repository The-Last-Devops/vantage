# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`last-monitor` is a self-hosted server & service monitoring system written in Rust,
combining Beszel-style host metrics (agent on each server) with Uptime-Kuma-style
service checks + alerting. Multi-user with namespace-scoped RBAC and public status pages.

## Architecture (the big picture — read this before designing changes)

Cargo workspace with three crates plus a hub-served SSR frontend:

- `crates/shared` — types exchanged between agent and hub (e.g. `MetricsReport`,
  `IngestAck`, the `API_KEY_HEADER` constant). Both sides depend on this; keep the
  wire format here so they can't drift.
- `crates/agent` — `last-agent` binary. Runs on each monitored server, collects host
  metrics via `sysinfo`, and **pushes** them to the hub. Configured purely by env vars
  (`HUB_URL`, `API_KEY`; `INTERVAL` is an optional override). The push cadence is
  controlled by the hub, which returns the next interval in `IngestAck`.
- `crates/hub` — the central Axum server: ingest endpoint, service probes, alert engine,
  auth/RBAC, JSON API, **and** the server-rendered web UI.

### Decisions that are load-bearing (don't silently reverse them)

- **Push model, not pull.** Agents reach out to the hub (`POST /pub/ingest`), authenticating
  with a per-server enrollment token in the `x-agent-token` header. This is what lets agents
  sit behind NAT/firewalls. The hub never connects back to agents.
- **Two separate PostgreSQL databases, two `PgPool`s in the hub:**
  - `config` DB — plain Postgres: users, namespaces, RBAC/membership, server & monitor
    config, alert rules, status pages.
  - `data` DB — Postgres **+ TimescaleDB extension**: metrics & heartbeat hypertables.
  - **Never JOIN across the two.** They relate only by IDs at the application layer
    (e.g. `server_id` stored in both). They may share one Postgres instance early on and
    be split onto separate hosts later by changing only connection strings — keep code
    agnostic to that.
  - Use TimescaleDB **continuous aggregates + retention policies** for downsampling instead
    of hand-rolling it.
- **Frontend is a Vue 3 SPA embedded in the hub binary** (in `frontend/`: Vite + Vue 3 +
  vue-router + Pinia, Tailwind via PostCSS, charts via **uPlot**). `vite build` emits to
  `frontend/dist`, which is embedded into the hub and served at `/`; any non-`/api/*` route
  falls back to `index.html` (SPA history fallback). The SPA talks to the hub's JSON API with a
  same-origin session cookie (`fetch(credentials:'include')`). Dev: run the hub on :8080 and
  `vite` on :5173 (proxies `/api`). Still a single binary — keep it that way; don't add a
  separate server for the UI. (Migrated from the original Rust SSR + HTMX; see `frontend/PLAN.md`.)
  - **Never paint a blank/black screen while loading** — see the "Frontend" convention below.
- **RBAC is namespace-scoped.** Permissions live in a `memberships` table (user × namespace ×
  role: `owner` / `editor` / `viewer`), plus a system-level `admin` (bypasses to owner
  everywhere). Authorize at the namespace boundary — every namespaced route funnels through
  `rbac::require_role`, the single place permission rules live.
- **Auth = opaque DB-backed sessions, not JWT** (revocable: logout/admin-revoke work without a
  blocklist). Random token in the `sessions` table, delivered as an httpOnly cookie; passwords
  hashed with argon2. The `CurrentUser` axum extractor resolves the cookie on each request.
  **No open registration** — the first admin is bootstrapped from `ADMIN_EMAIL`/`ADMIN_PASSWORD`
  env on startup, and admins provision further users via `POST /api/users`. Auth lives entirely
  in `auth.rs` so adding OAuth/OIDC/LDAP later only means minting a session there.
- **Three auth paths, don't conflate them:** agents authenticate per-request with the
  `x-agent-token` header (no session); humans use the session cookie; programmatic callers
  (scripts, third parties, the **MCP server**) send `Authorization: Bearer <pat>`. The
  `CurrentUser` extractor accepts cookie *or* PAT, so a PAT acts AS its user and inherits that
  user's RBAC — scope a token by issuing it to a limited service-account user. PATs live in
  `api_pats` (sha256-hashed, revocable), distinct from agent enrollment keys (`api_keys`).
- **MCP server is embedded in the hub** at `POST /mcp` (JSON-RPC 2.0, PAT-authed) — see `mcp.rs`.
  Tools run with the caller's RBAC: reads scoped to their namespaces, writes via `require_role`.
  Adding a tool = one arm in `call_tool` + an entry in `tool_defs`; it must enforce RBAC itself.
- **sqlx with runtime queries** (`sqlx::query` / `query_as`), not the compile-time `query!`
  macros, so the workspace builds without a live database / `DATABASE_URL` at compile time.

## Commands

```bash
# Build everything
cargo build

# Run the hub (needs CONFIG_DATABASE_URL + DATA_DATABASE_URL env vars)
cargo run -p hub

# Run an agent against a hub
HUB_URL=http://localhost:8080 API_KEY=<api-key> cargo run -p agent

# Tests
cargo test                      # whole workspace
cargo test -p hub               # one crate
cargo test -p hub <name>        # a single test by name

# Lint / format
cargo clippy --all-targets
cargo fmt

# Local stack (hub + Postgres/TimescaleDB)
docker compose up -d
```

## Conventions

- Agent ↔ hub wire types belong in `crates/shared` — change them there, never duplicate.
- Metrics/time-series writes target the `data` pool; everything else targets the `config` pool.
- Migrations are split per database (`migrations/config/`, `migrations/data/`); only the
  `data` DB runs `CREATE EXTENSION timescaledb`.
- **Checks/smoke-tests go in a committed `scripts/*.sh`, then run with `bash scripts/<name>.sh` —
  never ad-hoc one-liners (no inline `curl | python`, piped greps, etc.).** Write the check into a
  script, run it yourself, and don't ask for permission to run it. New checks should be idempotent
  and self-cleaning (e.g. `scripts/check-alerts.sh`).
- **Validate every user-supplied field server-side — the API is the source of truth.**
  The Vue SPA can be bypassed, so each create/patch handler must reject bad input with
  `400` *before* the INSERT/UPDATE, and store the trimmed value. Reuse the shared validators
  in `crates/hub/src/api/mod.rs`: `valid_name(s, max)` for display names (channel / monitor /
  system / status-page title), `valid_ns_name` for slugs & identifiers (lowercase, hyphen,
  no spaces — it's in URLs), `valid_email` for emails (ASCII, no whitespace). Mirror the same
  rule in the Vue form for instant feedback (e.g. the email regex in `Members.vue`). When you
  add or change a handler that accepts input, validate its fields in the **same** change — a
  weak check like `email.contains('@')` is how "kiên béo ngu dốt @gmail.com" got in.
- **Frontend: never paint a blank/black screen while loading — always show a loader.**
  - Route components in `frontend/src/router/index.js` are imported **eagerly**, not lazily
    (`() => import()`). A lazy route fetches its JS chunk on first navigation and the router renders
    nothing until it lands — that gap is the blank flash. Keep new pages eager-imported.
  - Every page that fetches data owns a loading flag initialised to its "still loading" value
    (`const loading = ref(true)` / `const loaded = ref(false)`) and renders `Loading…` until the
    first fetch settles — clear it in a `finally`, and also on the no-namespace early-return so it
    can't spin forever. Polling reloads must NOT re-flash the loader (gate the loader on the
    first-load flag, not on every fetch). Show the empty-state only *after* loading completes
    (`v-if="loading" … v-else-if="!items.length"`).
  - **Gate the whole content area behind the loaded flag**, not just an empty-state. If a page
    sets its data *before* `minLoad`'s minimum time elapses, an ungated list renders *under* the
    spinner and then jumps when the loader hides. Wrap the list/empty-state in `<template v-else>`
    of the `v-if="!loaded"` loader so they never show together.
- **Namespace-scoped data: aggregate across the selection AND show the namespace.** The sidebar
  selector is a multi-select in the URL (`?ns=a,b`; empty = all). A page listing namespaced data
  (alerts, events, channels-in-use, services, hosts) must show the union of **all** selected
  namespaces — never collapse to one. The bug pattern to avoid: `selectedNsName()` returning a
  name only when exactly one is picked and falling back to `namespaces[0]`; instead derive an
  `activeNs` set and fetch/merge per namespace (or filter a global list client-side, like
  Systems). Every row of merged data must be **labelled with its namespace** so it's clear which
  one it belongs to. The create/edit flow derives the target namespace from the chosen object,
  not from a single global selection.
- **List order must be stable across reloads — a mutation must never reorder rows.** Toggling a
  rule's `enabled` once looked like it toggled a *different* rule because the backend sorted
  `enabled DESC`, so the row jumped on reload. Sort lists client-side by a key independent of the
  mutated field (e.g. namespace → name → id), and dedupe by id defensively after a multi-source merge.
- **Secrets are shown only to those who can edit the owning resource.** Channel configs (tokens,
  passwords, webhook URLs), push-monitor tokens, and user-supplied request headers are credentials.
  List/detail endpoints must redact them (`notify::redact_secrets`, strip `push_token`, mask
  `Authorization`/`Cookie`/`x-api-key`) unless the caller is editor+ of the resource's namespace;
  never log them. When adding a field that can hold a secret, redact it in every read path.
- **Long-running binaries shut down gracefully.** The hub serves with
  `axum::serve(...).with_graceful_shutdown(shutdown_signal())` and the agent's loop `select!`s the
  interval against the same signal — both drain/stop on SIGTERM **and** Ctrl-C. Keep new
  background loops cancellable the same way so Docker/k8s stop them cleanly.
- **Mobile/responsive is not optional.** Use `h-[100dvh]` (not `h-screen`) for full-height
  panels so mobile browser chrome doesn't hide the bottom (it hid the namespace selector + logout).
  Multi-pane layouts stack on small screens (`flex-col md:flex-row`, `w-full md:w-[...]`); stat
  grids get breakpoints (`grid-cols-2 sm:grid-cols-4`); wide tables get an `overflow-x-auto` wrapper.
- **Clickable cards: the whole card opens the view; inner action buttons use `@click.stop`.**
  Don't make only a sub-region clickable — put `@click` on the card and `.stop` on every
  button/toggle inside it.
- **No native `title=` tooltips or `<select>` — use the themed primitives.** Both are
  registered globally in `main.js`. For a hover hint use the `v-tip` directive
  (`v-tip="'Edit'"` or `v-tip="expr"`, `frontend/src/lib/tooltip.js`) — it's faster and themed,
  unlike the slow/unstyled browser `title`. For a dropdown use `<UiSelect v-model=… :options=…/>`
  (`components/UiSelect.vue`); `:options` accepts strings, `[value,label]` pairs, or
  `{value,label}` objects, plus `block` (full-width) and `placeholder`. Don't pass a `title`
  attribute to DOM elements (the `title` prop on `<AppShell>` is the page heading, not a tooltip).
- **No native `window.confirm()`/`alert()` for confirmations — use `await confirm({…})`.**
  Import `{ confirm }` from `lib/confirm.js` (themed dialog, `<ConfirmDialog/>` is mounted once in
  `App.vue`): `if (!(await confirm({ title, message, danger: true, confirmText: 'Delete' }))) return`.
  It returns a `Promise<boolean>`; the enclosing handler must be `async`. Use `danger: true` for
  destructive actions (red + warning icon).
- **Dev loop: stop the running hub before `cargo build`/`cargo run -p hub`** — a running binary
  holds `target/debug/last-hub` and the link fails (looks like an obscure linker error). Free
  `:8080` first. Watch disk too: a full disk surfaces as `ld: No space left on device`.
- **Reclaim disk with `bash scripts/disk-cleanup.sh`** — `target/` alone grows to ~15-18 GB and
  is the usual cause of a full disk. The script runs `cargo clean` + prunes Docker **build cache**
  (never volumes/DB) + removes stray release tarballs. Safe to run anytime (build output is
  regenerated). For unattended machines, add the weekly cron shown in the script's header.
- **Always run `cargo fmt` immediately before committing Rust** — CI's Format job runs
  `cargo fmt --check` and has failed a release twice because new code was committed unformatted.
  Run `cargo fmt && cargo fmt --check` after the *last* Rust edit, not before it.
