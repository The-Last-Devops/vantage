# Vantage — Project Brief

A handoff/context document: what the product is, its design language, components,
and conventions. Paste any section into another assistant (e.g. Claude design
system) or share with a new teammate.

---

## 1. Overview

**Vantage** — a self-hosted, **centralized DevOps control plane**: one place to
**manage and watch** your whole infrastructure — servers, clusters, services, and cloud.

**What it manages**
- **Servers** — host metrics (CPU / memory / disk / network…), agent pushes to the hub; SSH / shell exec to act on a host.
- **Clusters** — Kubernetes / Docker fleets.
- **Services** — uptime / health checks.
- **Cloud** — cloud-hosted resources.

**What it does across all of them**
- **Monitor** — a single, clear overview of everything's health.
- **Alert** — notify the moment an incident or a condition that needs attention occurs.
- **Operate** — act on problems (exec / run) right from where they surface.

**Why**
- **Detect problems fast** the moment an incident hits.
- Give the **clearest end-to-end overview** to both **prevent** and **resolve** incidents.

→ It's a **DevOps console / control plane** (monitor **+** alert **+** operate), not a read-only dashboard.
Display name: **"Vantage"** (Title Case).

**Design implications:** surface what needs attention first (attention-first), make
state readable at a glance (pill / dot / severity), and offer a path to act / exec
right where the problem shows up.

## 2. Stack (design-relevant)

Vue 3 SPA (Vite, vue-router, Pinia) embedded in a single Rust hub binary · **Tailwind** ·
tokens = **CSS vars** (`:root` = dark by default, `.light` class) · charts = **uPlot**.

## 3. Color tokens (source of truth: `frontend/src/style.css`)

**Dark (default)** — bg `#0B0E14` · surface `#10151F` · surface2 `#141A26` · line `#1E2632` ·
fg `#E2E8F0` · muted `#A9B6C8` · faint `#7C8A9C` · **accent `#34E1C4`** (teal) ·
ok `#34D399` · warn `#FBBF24` · down `#F87171`

**Light (`.light`)** — bg `#F6F7F9` · surface `#FFFFFF` · line `#E4E7EC` · fg `#0E1726` ·
muted `#46566B` · faint `#647386` · **accent `#0D9488`** (teal-600) ·
ok `#15A34A` · warn `#C2890B` · down `#DC2626`

State colors flip with the theme; semantic (ok/warn/down) is **separate** from the accent.

## 4. Typography

- **Inter** = UI text · **JetBrains Mono** = data / identifiers (tabular-nums).
- Scale: metric 40·800 / stat 28·800 / h1 20·700 / h2 16·600 / body 14 / label 12·700 uppercase / data mono 13.

## 5. Logo / brand

Mark = a **summit `^` next to a terminal cursor block `_`** in teal on a rounded tile,
knockout stroke `#08231F`. A vantage point you *watch from* and a prompt you *operate
from*. **Locked.** Lives in `assets/logo.svg`, `assets/logo-mark.svg`,
`frontend/public/favicon.svg`, and the sidebar in `AppShell.vue`.

## 6. Frame / layout standard

Left sidebar + header. **Active nav = bold fg text + a 3px teal left bar** (NOT a teal
background), **no dividers** between groups. Header = **breadcrumb (left)** +
(workspace filter · theme · version · avatar) on the **right**. Full-height (`100dvh`),
responsive (panes stack on mobile).

## 7. Components

- **Existing**: `DataTable`, `StatePill`, `UiSelect`, `ConfirmDialog`, `PageLoader`, the `v-tip` directive.
- **Planned**: `Page / PageHeader / PageBody / PageFooter / PageRail`, `UserMenu`,
  `WorkspaceFilter`, `FleetChart` (overlaid multi-host + p5–p95 band), host `Table / Map / Grid`, time-range picker.

## 8. UX rules (non-negotiable)

- **Never paint a blank screen while loading** — always a loader; navigation uses a
  **stale-while-revalidate cache** (paint last data, refetch in the background).
- **Tables**: every column at full-strength color (all data matters), **clickable cells
  use the accent color**, **bold header**; tables carry a filter + bulk-action toolbar.
- **No native `<select>` / `title=` / `confirm()`** — use the themed primitives
  (`UiSelect`, `v-tip`, `confirm()`).
- Workspace-scoped data must **aggregate across the whole selection** and be **labelled
  with its workspace**; **list order is stable** (a mutation never reorders rows).
- **Redact secrets** (tokens / webhook URLs / request headers) unless the caller is
  editor+ of that workspace.
- Validate every user field **server-side** (the API is the source of truth).

## 9. Open decisions

- Whether to **adopt Inter + JetBrains Mono in the actual app** (self-host once) so the
  app matches the gallery. (The app still uses system fonts; the gallery uses Inter + JBMono.)
