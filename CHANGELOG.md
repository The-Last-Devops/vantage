# Changelog

All notable changes to **Vantage** are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each released version's section is used verbatim as the GitHub Release notes
(extracted by `.github/workflows/release.yml`), so keep entries user-facing.

## [Unreleased]

## [2.3.14] — 2026-07-08

### Added
- **Kubernetes cluster monitoring — CPU/memory per container, aggregated on read.**
  The cluster agent now also queries metrics-server (`metrics.k8s.io`) and reports one
  row **per container** with full pod metadata (namespace, node, phase, owning workload
  resolved pod → ReplicaSet → Deployment, restart count, and all pod labels). The hub
  stores these in a new `kube_container_stats` hypertable and **aggregates on read**, so
  a new **Cluster** page (`/cluster/:id`) can break usage down by **namespace, workload,
  or any label**, chart CPU/memory over time for the whole cluster or a selected group,
  and drill down to individual pods/containers. Usage reads 0 (metadata still collected)
  on clusters without metrics-server.

## [2.3.13] — 2026-07-08

### Fixed
- **Upgrades no longer break on existing databases.** The 2.3.12 "namespace → workspace"
  rename rewrote already-applied migrations in place, so a hub with a pre-existing config
  database refused to start (`migration 1 was previously applied but has been modified`).
  Migrations `0001`–`0017` are restored to their original form (checksums match again) and
  the rename now ships as a new forward migration (`config/0027`), so both fresh and
  existing databases converge on the workspace schema with no manual intervention.

## [2.3.12] — 2026-07-08

### Added
- **Docker Compose auto-update (opt-in)** — a Watchtower service (behind the `autoupdate`
  profile) that recreates the hub/agent containers when their image tag updates.
- **Memory breakdown + Swap charts on the host page** — the agent now reports the Linux
  `free`-style memory split (available / buffers / cached / free) from `/proc/meminfo`, and
  the node detail view adds a "Memory breakdown" chart and a "Swap" chart (shown when the
  host has swap). Existing "Memory used %" is unchanged.
- **Kubernetes cluster stats (agent)** — a new cluster-scoped agent collector queries the
  kube-apiserver (read-only, in-cluster ServiceAccount) and pushes per-namespace and
  per-deployment stats (pod phases, restarts, replica health) to the hub, which stores them
  as TimescaleDB hypertables. Cluster dashboards land in a later release.

### Changed
- **"Namespace" RBAC concept renamed to "Workspace"** across the UI and JSON API — the
  workspace selector, membership roles, and status-page scoping now read "workspace".
- **Alerts & notify** — search and New moved into a left toolbar, matching the
  Infrastructure/Services layout.
- **Services "Down" view rebuilt** — shows an all-clear banner with 30-day stats when
  nothing is down, or a Down-now table plus recent downtime otherwise.

### Fixed
- About page no longer scrolls a short changelog inside a tiny box (cap raised to 70vh).
- Auto-update degrades gracefully on raw k8s manifests: when the pod lacks the rollout
  RBAC (`HUB_DEPLOYMENT_NAME` + ServiceAccount), it falls back to an in-place restart
  instead of silently not updating.
- Byte-unit chart Y-axis labels (e.g. "10.5 M/s") are no longer clipped (wider gutter).
- Config-stats query no longer mangled by the workspace rename.

## [2.3.11] — 2026-06-30

### Added
- **Configurable cleanup for the config database's log tables** — Data & retention now
  lets admins set how long the growing log tables are kept (defaults: SSH session
  transcripts 30 days, alert events 365, shell sessions 365, login sessions 14); a
  background job prunes older rows. Each config table also shows what it's for.

### Changed
- **Data & retention** now shows both databases laid out in balanced two-column tables;
  config-DB tables report exact row counts (was a planner estimate that read 0 for
  rarely-written tables) and a one-line purpose for each.
- Hub-wide settings moved from one-column-per-setting to a key→value `settings` table, so
  adding a setting no longer needs a schema migration (existing settings migrated in place).

## [2.3.10] — 2026-06-30

### Added
- **Data & retention now covers both databases** — the page shows the time-series
  database (TimescaleDB) and the configuration database (PostgreSQL) side by side, each
  with its size and per-table breakdown.
- **Storage cap for the time-series database** — set a size ceiling and, optionally, let
  the hub auto-evict the oldest data when usage goes over it (off by default; drops the
  oldest time chunks first across every tier). Default ceiling 10 GB.

### Changed
- **Service Uptime and Trend now reflect the last 24 hours** — the Uptime column header
  reads "Uptime 24h" and the trend sparkline shows one bar per hour over the past day.
- **Zero-downtime auto-update** — when a new image is published the hub now triggers a
  rolling redeploy of itself (new pod up and ready before the old one drains) instead of
  exiting in place, which used to cause a brief 503 on every update.
- **Status tiles filter by status** — clicking Down / Critical / Warning on the Overview
  now opens the host list filtered to just that status; the page was renamed from
  "Needs attention" to "Issues".
- **New / Edit service form redesigned** — a single centred column grouped into Basics /
  Schedule / HTTP options / Meta sections, with Name as the headline field.
- **Tidier, calmer layouts** — page width is capped on very wide screens; the Services
  summary is a slim figure strip; the recent-events panel fills the available height with
  its own scroll and a pinned pager; dropdowns now show their full list instead of
  clipping at ~7 rows; the Services and Infrastructure search boxes match and the Add
  button sits beside them.
- **Distinct sidebar icons** — every menu item now has its own meaningful icon.
- Hand-built tables across Infrastructure, Backup, Audit, SSH keys, Data & retention and
  API tokens now use the standard table-header style; machine identifiers are monospaced.
- The SSH console terminal re-themes instantly when you toggle light/dark.

### Fixed
- **Passkeys work with no extra configuration** — the relying party is derived from the
  page's own address, so passkeys register and sign in on whatever domain serves the hub
  without setting `WEBAUTHN_RP_ID` / `WEBAUTHN_ORIGIN` (those remain optional overrides).

## [2.3.9] — 2026-06-29

### Changed
- **Login prioritises passkeys** — when you have a passkey registered and the browser
  supports it, sign-in prompts the passkey automatically; the authenticator code and
  button remain as a fallback.
- **UI synced to the design system** — Inter is now the interface font and JetBrains
  Mono is reserved for data (numbers, identifiers, timestamps, logs). Status colours use
  semantic tokens so they flip correctly with the light/dark theme, and light-theme text
  contrast is improved. No layout or spacing changes.
- **Overview tidy-up** — slightly larger tile labels, and the Security tile now shares a
  row with the admin system stats ("Account & system") instead of sitting alone on a
  near-empty row.

### Fixed
- Clearer error when a passkey fails to register because `WEBAUTHN_RP_ID` /
  `WEBAUTHN_ORIGIN` don't match the page's domain — it now tells you exactly what to set.
- `PUBLIC_URL` is **auto-detected** from the incoming request (`X-Forwarded-Proto`/`Host`)
  when unset, so the exposure self-check works behind a reverse proxy without manual
  config. The relevant env vars are now wired into Docker Compose and the Helm chart.

## [2.3.8] — 2026-06-29

### Added
- **Overview redesigned** as a single, uniform grid of clickable status tiles — Hosts
  (total/up · down · critical · warning), Services (total/up · down · avg uptime),
  Operations (alerts firing · events 24h · agent updates · backup) and Security
  (two-factor) plus, for admins, a System row (database size · workspaces · members).
  Each tile links to its page, **turns green when healthy** and red/amber when it needs
  attention. The scattered event feed / incident list / capacity charts are gone.
- **Public-exposure self-check** (**Settings → Security**, admin) — the hub probes its own
  `PUBLIC_URL` at an unauthenticated marker outside `/pub`; if it's reachable with no auth
  gate it warns you to front the hub with nginx basic-auth / Cloudflare Zero Trust (allowing
  `/pub/*` through) and links the docs.

### Docs
- README gains a full **environment-variable reference** (hub + agent) and a consolidated
  **Security** section; new [docs/exposure.md](docs/exposure.md) covers gating a public hub.

## [2.3.7] — 2026-06-29

### Security
- **Login brute-force throttle.** After 5 failed attempts an account is locked out for an
  escalating cooldown (HTTP 429) — this covers password *and* TOTP / backup-code guessing,
  so 2FA can't be brute-forced online. Cleared on a successful sign-in.
- **TOTP codes are single-use within their time window** — a code (or backup code) accepted
  once can't be replayed.
- **Email (SMTP) notify channels go through the SSRF guard** like every other channel, so a
  channel can't be pointed at loopback/link-local/internal hosts to probe the network.

### Fixed
- **Backups now include PATs, SSH keys and passkeys** (and app settings) — restoring a backup
  no longer silently destroys API tokens, stored SSH keys and registered passkeys.
- Concurrent first-time console step-ups can no longer race and orphan a user's SSH keys.

## [2.3.6] — 2026-06-29

### Added
- **Two-factor authentication.** Opt-in from **Settings → Security**:
  - **Authenticator app (TOTP)** — scan a QR (or enter the setup key) with Google
    Authenticator / 1Password / Authy; sign-in then asks for a 6-digit code. One-time
    **backup codes** are issued for recovery.
  - **Passkeys (WebAuthn)** — register Touch ID / Windows Hello / a security key and
    assert it at sign-in. Add/remove multiple passkeys.
  - Sign-in offers a passkey and/or a code; either satisfies the second factor.

### Changed
- **Settings → Security** is now a focused single-open accordion (Password, Two-factor,
  Passkeys, SSH keys), full-width.
- The **Services** page mirrors Infrastructure: overview KPIs first, then a toolbar with
  search on the left and **Add service** on the right.
- The **login page** shows the brand mark and explains *why* a sign-in failed (API
  unreachable, timeout, 5xx, plus the server's message) — it's an internal console.
- Long, time-growing lists are capped + scroll + paginated (recent events, alert events,
  service down-history); the **Audit** table is more compact and no longer overflows.

### Security / Fixed
- The **SSH console** warns before you close the tab or navigate away while a session is
  live, and no longer falsely shows "Shell is disabled". A reachable host over a critical
  threshold reads as **Critical** (not **Down**, which now means unreachable).
- A long-lived browser tab is told when a newer build is deployed (reload banner).

## [2.3.5] — 2026-06-29

### Security
- **SSH private keys are now envelope-encrypted.** Each user has one master key that
  seals their keys; it's wrapped inner by their password and outer by an application
  secret `EXEC_APP_SECRET` (env/KMS, never in the DB). A database leak alone can't
  unwrap anything, changing a password no longer orphans keys, and the app secret is
  rotatable without any passwords (`vantage-hub rotate-app-secret`). See the README.
- **Change your own password** from the new **Settings → Security** page — your SSH
  keys keep working (they're re-secured under the new password). Two-factor auth is
  on the way (the TOTP verification core has landed).

### Fixed
- **The SSH console no longer says "Shell is disabled for this host."** The leftover
  per-host enable/disable flag is gone — every host is SSH-capable, gated only by your
  workspace `can_exec` permission and a live agent tunnel.

### Changed
- A reachable host that's over a **critical** threshold (e.g. disk 93%) now reads as
  **Critical** (orange), not **Down** — "Down" is reserved for hosts that are actually
  unreachable.
- New **Settings → Security** page; the account menu links to it instead of a modal.
- Every sidebar submenu item now has an icon. The host's SSH port moved into a small
  gear (rarely changed). Service **Down history** is capped at ~2/3 of the screen, with
  scroll and pagination.
- Open a long-lived tab through a deploy and a **"new version available — Reload"**
  banner now appears instead of silently running stale code.

## [2.3.4] — 2026-06-29

### Changed
- **The SSH console now opens in a new browser tab and lives inside the app frame**
  (the sidebar and header stay visible) instead of taking over the whole screen, so
  you can keep a dashboard open in the original tab while you work in the terminal.
- **The terminal opens at its real, fitted size and tracks live resizes**, so `htop`,
  `top`, and other full-screen TUIs fill the whole window instead of a cramped 80×24 box.
- **Open a console straight from the host page** — an "SSH" button now sits next to the
  host's Up/Down status (shown when you have shell access and the agent's tunnel is live).
- **Dropped the per-host enable/disable shell toggle** — the shell is always available;
  the host's Shell card now just shows the SSH port and tunnel status.

### Fixed
- The version badge in the header is green when you're on the latest release (and amber
  when an update is out), instead of a plain grey pill.

## [2.3.3] — 2026-06-29

### Fixed
- **Hosts no longer flap down→up.** The "down" threshold sat too close to the agent's
  60s push interval (and 2.3.2 had dropped it to 15s, which marked healthy hosts offline
  most of the time). It's now 120s — a full interval of headroom, so a host never grazes
  the boundary just before its next push.

## [2.3.2] — 2026-06-29

### Fixed
- **Shell tunnel now works with an `http://` HUB_URL behind TLS.** The agent followed the
  http→https redirect for metrics but not for the shell tunnel, which looped on
  "301 Moved Permanently"; it now upgrades `ws→wss` on the redirect.
- The host console header showed the host's UUID — it now shows the real name.
- The SSH-user / key-name fields no longer get auto-filled with your account email.

### Added / Changed
- **Shell is enabled by default on the agent** (set `ALLOW_SHELL=0` to opt out), so an
  upgraded agent opens the tunnel without a config change.
- **SSH keys accept any standard format** (OpenSSH, PEM RSA, PKCS#8), encrypted or not —
  with an optional key passphrase. Uploading a key file no longer displays its contents,
  and add-key errors now say exactly what's wrong.
- A host is marked **down only after 15s** of silence (brief blips no longer flip it).
- Refined Overview (summary + capacity up top, paged events below), the unified header
  (compact ⌘K, lighter borders), and detail-page breadcrumbs.

## [2.3.1] — 2026-06-29

### Fixed
- The header no longer appears empty on pages that show their own hero (e.g. **Fleet**) —
  the top bar always names the current page now (every route provides a title fallback).

## [2.3.0] — 2026-06-29

### Changed
- **Unified top bar.** The workspace switcher and your account menu moved up from the
  sidebar into a single 56px header — alongside a global search (⌘K), alerts, theme
  toggle, version, and avatar. The sidebar is now just brand + navigation. Picking
  workspaces works exactly as before (`?ws=` in the URL).
- **Redesigned Services.** The list gains a health KPI strip and a severity-aware table
  (down/degraded rows washed) with uptime, latency, and a trend sparkline per check,
  plus a related live events feed. A service's detail page leads with a status hero and
  per-window uptime/latency KPIs, with its checks, history, and event log in clean cards.

## [2.2.0] — 2026-06-29

### Added
- **New Overview dashboard** — an attention-first landing: open incidents lead, then a
  health KPI strip, with the fleet CPU trend demoted to the bottom.
- **Fleet war-room** (`Fleet`) — every host & service at a glance: a workspace-grouped
  health heatmap, services with uptime, live incidents with inline SSH/Exec, and top-load.
- **Fleet metrics** (`Metrics`) — small-multiples across the fleet (CPU / Memory / Disk /
  Network), one line per host; click a host to isolate its line across every panel, with
  1h/6h/24h/7d ranges.
- **SSH key library** — manage your own SSH keys under **Settings → SSH keys**; they're
  encrypted with your account password and reusable across hosts.

### Changed
- **Opening a console now lets you choose how to authenticate** — your SSH user with a
  host password, or a key from your library. SSH keys moved from per-host credentials to
  your account (one library, used everywhere).
- **Refreshed interface** — a new icon set, a tidier sidebar + 50px header, and
  severity-aware host tables (row washes + a left rail, utilisation bars coloured by
  threshold, and inline per-row operate actions).

## [2.1.1] — 2026-06-28

### Added
- **Grant shell access to members from the UI.** In the member editor, each workspace
  where someone is an **owner** now has a **Shell access** toggle that grants/revokes
  the console capability (previously only system admins could open a shell). A `shell`
  chip marks members who have it.

### Fixed
- The member **reset-password** form now enforces the same strong policy as everywhere
  else (12+ characters, mixed) instead of the old 6-character minimum, and its generator
  produces a compliant password.

## [2.1.0] — 2026-06-28

### Added
- **Interactive SSH console (shell into a host from the browser).** Open a real
  terminal to a monitored host, routed through the agent — no inbound access to the
  host required. Security-first: it's **off by default** and opt-in on both sides (the
  agent must run with `ALLOW_SHELL=1` **and** an owner enables shell on the host).
  Opening a shell needs a dedicated **exec** capability (separate from "edit config")
  plus a **step-up password**. **Each person uses their own SSH key and their own
  account on the host** — the hub only stores your key for you, **encrypted with your
  own password** (no server master key; it can't be read without you). Every session is
  recorded to an immutable audit trail (on-screen output, never your keystrokes, so
  typed passwords aren't captured). `sudo` works as your host account allows. _(Live
  terminal resize and the audit-viewer UI are follow-ups.)_
- **Auto-update is an explicit opt-in.** The Helm charts (hub + agent) take an
  `autoUpdate` flag (default `false` = pinned/manual); `true` switches to the
  `:auto-update` image and sets `AUTO_UPDATE=1`. The **Add System → Kubernetes** flow
  has an **Auto-update** toggle that adds `?autoupdate=1` to the rendered DaemonSet
  manifest. Not everyone wants auto-update, so it's off unless chosen.

### Changed
- **Stronger password policy.** New and changed passwords must be 12–128 characters,
  mix at least three of {lowercase, uppercase, digit, symbol}, and avoid common or
  predictable choices (was: a 6-character minimum). Existing sign-ins are unaffected;
  the member forms show live guidance and the password generator meets the policy.

### Fixed
- About page: when running a pre-release build ahead of the latest GitHub release it
  now shows "Running a pre-release (vX) — ahead of …" instead of wrongly claiming
  "you're on the latest (older-tag)". Build sha is displayed short.

## [2.0.3] — 2026-06-28

### Added
- **Opt-in auto-update for Kubernetes** via a new rolling image tag **`:auto-update`**
  (the "auto" release channel). The hub advertises its build id in every `IngestAck`;
  an agent running `:auto-update` under k8s (`imagePullPolicy: Always`) that sees a
  newer hub build waits a per-host jitter, then exits so k8s re-pulls the new image.
  No binary download — the update is a normal registry pull (no new RCE surface).
  `:main` and pinned `:X.Y.Z` never self-update; `AUTO_UPDATE=0` is a kill switch.
  The **hub** on `:auto-update` also self-updates: it polls ghcr for the rolling
  tag's digest and exits when it changes (k8s re-pulls), so the whole stack stays
  current from one push — agents then follow the hub's new build id. CI always
  rebuilds both hub and agent for the `auto` channel so their build ids never skew.
- The agent now sends a branded `User-Agent: vantage-agent/<version>` when pushing.

## [2.0.2] — 2026-06-28

### Security
- **Monitor credentials no longer leak to viewers.** The monitor list/detail
  endpoints now redact secret `config` (auth headers like `Authorization`/`Cookie`,
  basic-auth/bearer credentials, Redis password) and mask the password in DB
  connection-string `target`s for anyone who isn't an editor of the monitor's
  workspace. (Previously only `push_token` was stripped, so read-only members could
  read stored credentials.)
- **SSRF egress guard on all outbound requests.** Service probes, notification
  webhooks, and the S3 backup endpoint resolve the target and reject loopback,
  link-local / cloud-metadata (`169.254.169.254`), CGNAT, and other reserved
  addresses — including IPv4-mapped forms and redirect hops — before connecting.
  This closes a read-SSRF where an editor could point an HTTP monitor at the cloud
  metadata endpoint and read the response back via the debug view. Private
  (RFC1918/ULA) targets stay allowed so internal monitoring works; set
  `EGRESS_POLICY=strict` to block those too.
- **Agent enrollment keys masked for viewers.** The workspace key list only shows
  the full `x-api-key` to editors; viewers see a short non-usable preview.
- **Session cookie now sets the `Secure` flag** (set `INSECURE_COOKIES=1` to keep
  plain-http local dev working).

### Ops
- Image builds retry transient crate downloads (`CARGO_NET_RETRY=10`) — a flaky
  `curl failed` while fetching a crate had failed one image job.

## [2.0.1] — 2026-06-27

### Changed
- **Internal: modularized the codebase — no functional changes.** Split the largest
  files into per-concern modules so features are easier to add / remove / upgrade:
  the hub (`notify`, `api/alerting`, `web/systems`, `web/monitors`, `probe`, `backup`),
  the agent (`collect` / `push`), and the five largest Vue pages (into presentational
  components plus a `hostFilter` lib). Public APIs, behavior, and the UI are unchanged.
  Removed dead HTMX/SSR leftovers. Added a priority-ordered "Guiding principles"
  section (security-first, lightweight, modular) to the contributor docs.

## [2.0.0] — 2026-06-27

**Vantage** — the project formerly known as "Last Monitor". This is a clean,
**incompatible** rebrand: there is no in-place upgrade from 1.x. Deploy fresh.

### Changed
- **Renamed everything to "Vantage".** Display name, repo, Rust crates/binaries
  (`vantage-hub`, `vantage-agent`), Docker images
  (`ghcr.io/the-last-devops/vantage-hub` / `vantage-agent`), Helm charts, Kubernetes
  resources, and the databases (`vantage_config`, `vantage_data`). No abbreviations.
- **New logo** — a summit `^` next to a terminal cursor: a vantage point you watch
  from and operate from.
- Positioned as a **centralized DevOps control plane** — manage and watch servers,
  clusters, services, and cloud; monitor, alert, and operate from one place.

### Breaking
- Image names, database names, and Kubernetes resource names all changed from the
  `last-monitor` / `lastmon` / `last-*` scheme. Existing 1.x deployments must be
  **recreated** with the new names (no data migration path is provided).

### Removed
- Dead server-rendered (HTMX) assets left over from the earlier SPA migration.

## [1.7.3] — 2026-06-27

### Changed
- **Switching pages is now instant.** Every list page used to re-show its loading
  spinner and re-fetch from scratch on each visit; pages now paint the last-known
  data immediately and refresh in the background, so navigating between Systems,
  Alerts, Monitors, Notifications, Members, Workspaces, Events, Audit, API tokens,
  and Data retention no longer flashes a spinner. The spinner appears only on a
  genuine first load or the first time you view a new workspace selection.

### Fixed
- **Cached data can't silently go stale.** Pages re-validate when the browser tab
  regains focus or the network reconnects, and never display a snapshot older than
  60 seconds without refreshing — so a long-open page won't show outdated values.
  Cached data is also cleared on logout.

## [1.7.2] — 2026-06-26

### Changed
- **Alert notifications are now properly formatted** — a title plus Type / Workspace /
  Condition / Detail / When, rendered natively per channel (Discord embed, Slack &
  Mattermost colored attachment, Telegram HTML, webhook structured JSON, email HTML,
  Matrix formatted) instead of a single terse line.
- **Confirmations use an in-app themed dialog** (Esc/Enter/click-outside, red for
  destructive actions) instead of the browser's plain `confirm()` box.
- The **Recent events** feed is more compact (it's secondary on the Services page).
- The README now shows an **architecture diagram**, and tooltips wrap long content.

### Fixed
- The alert rule **on/off toggle** now takes effect (it was flipping a table-row copy
  instead of the underlying rule).

### Ops
- Added `scripts/disk-cleanup.sh` (reclaim `target/` + Docker build cache, never
  volumes) with a weekly-cron snippet, documented in CLAUDE.md.

## [1.7.1] — 2026-06-26

### Added
- **Agent auto-upgrades http→https.** If `HUB_URL` is `http://` but the hub sits
  behind TLS and redirects, the agent now retries over https automatically — but
  only when the redirect stays on the **same host**, so it never follows elsewhere
  and can't leak its enrollment token. (Fixes k8s agents stuck on a 301.)
- **Brand: a real logo** (uptime-pulse mark) for the favicon, GitHub avatar, and a
  README banner. The favicon is now fixed (the sidebar logo still cycles hue).
- Lots more unit tests (validators, RBAC roles, secret redaction, the agent's
  redirect-upgrade guard).

### Changed
- **Services is now a sortable table** with bulk actions, and its create/edit form
  moved to a full page (matching alert rules & channels). An alert rule's **source
  is now editable** (re-targets in place). Workspace members are added from a
  **picker of existing users** instead of a free-text email box.
- Service rows show the **time window** their uptime/history covers; the alert-rule
  editor shows each channel's provider icon.
- Unified the page header (breadcrumb/title in one fixed-height bar; the sidebar
  highlights the right item on detail pages); bolder nav in the light theme.

### Fixed
- A brand-new alert rule that's healthy no longer shows **"Pending" forever** — its
  state is now recorded on the first evaluation.
- **CI/release only rebuild the image whose code changed**; an unchanged component
  is re-tagged from the previous release instead of rebuilt.

## [1.7.0] — 2026-06-26

### Added
- **Workspace-wide alert rules.** One rule can watch **all services** or **all hosts** in a
  workspace (new hosts/services are covered automatically) — no more editing a rule per target.
- **Cross-linking everywhere.** A service or host detail page lists the **alert rules** covering
  it; a notify channel's page shows the **rules that use it** and the **services/hosts it reaches**.
- **Test a channel before saving it** — send a test from the unsaved config while creating or
  editing, instead of having to save first.
- **Audit log filters, pagination, and retention.** Filter by user/endpoint/object, HTTP method,
  and result (success / client / server); page through history; and set how long the log is kept
  (admin), with old rows pruned automatically.

### Changed
- **Rancher-style clarity pass.** Higher-contrast light theme, and the list pages
  (**Services, Alert rules, Notify channels, Members**) are now sortable, filterable tables with
  status pills and bulk actions (select rows → enable/disable/delete). Bulk buttons stay visible
  (disabled until you select) so the actions are discoverable.
- **Workspaces redesigned** into a card grid; each workspace has a detail page to manage its
  **members**, see its **attached alert rules**, and tuck the "Needs attention" thresholds away.
- **Alert rules, channels, and services now edit on a full page** (no more modal); a service's
  create/edit form moved out of the list into its own page.
- **Themed tooltips and dropdowns** replace the slow native `title` hover and the unstyled
  `<select>`, matching the app in both themes.
- The page **breadcrumb moved into the header bar** (saves a row), and the sidebar now highlights
  the correct item when you're on a detail/editor page.

## [1.6.0] — 2026-06-25

### Added
- **Programmatic API access tokens (PAT).** Mint tokens under **Settings › API tokens**
  (shown once, revocable, optional expiry) and call the API with
  `Authorization: Bearer <token>`. A token acts as its user and inherits that user's
  RBAC — scope it by issuing it to a limited service-account user.
- **Embedded MCP server** at `POST /mcp` (JSON-RPC 2.0), authenticated by a PAT, so AI
  assistants (Claude, etc.) can read and operate the monitor. Tools: `list_systems`,
  `list_services`, `alerts_firing`, `recent_events` (read, scoped to your workspaces) and
  `run_service_check`, `toggle_alert_rule` (write, require editor of the target's workspace).

## [1.5.5] — 2026-06-25

### Added
- A newly created service is **probed immediately**, so its status (and any alert on
  it) shows at once instead of waiting for the next scheduler cycle.
- Viewing a notify channel lists the **alert rules that use it** (with their workspace
  and enabled state) — click one to jump to the rule.
- Workspace-scoped data now **shows its workspace**: alert rules, the events feed, and
  the services list are each labelled with the workspace they belong to.

### Changed
- **Graceful shutdown** — the hub drains in-flight requests and the agent stops cleanly
  on SIGTERM / Ctrl-C (Docker/k8s can stop them within the grace period).
- Click anywhere on a rule or channel card to open it (action buttons excepted).

### Fixed
- Toggling an alert rule no longer looked like it toggled a *different* rule — the list
  now keeps a stable order instead of re-sorting on every change.
- The Rules list no longer renders under the loader and jumps up when loading finishes.
- **Mobile**: the Services two-pane layout no longer overflows the screen (it stacks),
  and the sidebar's workspace selector + logout are no longer hidden by the browser bar.

## [1.5.4] — 2026-06-25

### Security
- Channel secrets (tokens, passwords, webhook URLs) are masked for anyone who
  can't edit the channel; only editors of the channel's workspace see them.
- Push-monitor tokens are no longer returned to viewers — shown only to editors
  on the monitor detail page (the token is a write credential).
- Credential-bearing request headers (Authorization / Cookie / API-key) are
  redacted in the monitor debug view.

### Added
- **Notify channels are a shared resource**: every workspace can see and use any
  channel in its alert rules; only an editor of a channel's own workspace can
  edit or delete it. New `GET /api/channels`.
- Click a rule or channel card to view it (read-only when you lack edit rights).

### Changed
- Alert **Rules** and **Events** now span all selected workspaces — previously the
  list silently collapsed to a single workspace when "all" or several were picked.

### Fixed
- Mobile: the workspace selector and logout were pushed below the browser toolbar
  in the sidebar (now uses dynamic viewport height).

## [1.5.3] — 2026-06-25

### Added
- **Discord** notify channels can post into a specific **thread** — a new optional
  "Thread ID" field on the Discord channel form.

### Fixed
- **Services** now respects the sidebar workspace filter — the service list, the
  Up/Down/Paused/Total stats, and the Recent-events feed all scope to the selected
  workspace(s), matching Infrastructure (previously Services ignored the filter).

## [1.5.2] — 2026-06-25

### Added
- **Members** redesign — search + role filter, an *Add member* dialog and an edit
  slide-over, per-workspace access shown as named role chips, and a clearer legend
  distinguishing system roles from workspace roles.
- **Services › Recent events** now shows a message on recovery ("Recovered") and a
  **Duration** column — how long each state lasted, with the latest marked "ongoing".

### Changed
- The Services uptime/heartbeat bar now spans the full width of each card.

### Fixed
- **Input validation across the API.** Every create/edit now rejects junk server-side:
  malformed emails (e.g. with spaces), blank or over-long names (channels, monitors,
  systems), and invalid status-page slugs can no longer be saved.

## [1.5.1] — 2026-06-25

### Changed
- **Services**: the "Add monitor" button and related labels now read **"service"**
  (Add service / New service / Edit service / "No services yet"), matching the nav.
- Releases now ship with curated notes from this `CHANGELOG.md`.

### Added
- The sidebar logo and **"Last Monitor"** name now link to the home page.

## [1.5.0] — 2026-06-25

### Added
- **Multi-channel alerts** — one rule can fan out to several notify channels.
  17 providers: Telegram, Slack, Discord, Mattermost, Teams, Google Chat, Matrix,
  ntfy, Pushover, Gotify, Bark, PagerDuty, Opsgenie, Twilio SMS, SMTP email,
  generic webhook, and Apprise — each with a data-driven config form and a one-click test.
- **Re-notify cadence** for a still-firing alert (configurable; replaces the old fixed cooldown).
- **Alert › Events** — an incident history feed.
- **Scheduled (cron) backups to S3.**
- **Version badge** in the top bar (green = up to date, amber = a newer release is out),
  linking to the About page.

### Changed
- **Alerts UI overhaul** — live firing state, history, and inline toggle/test/edit.
- **Notify channels** reworked — test button and inline edit; email folded into the SMTP provider.
- **Loading UX** — pages are eager-loaded so navigation never flashes a blank/black screen,
  and a centered spinner with a minimum display time replaces the old top-left "Loading…".
- **Audit log is human-readable** — shows the action and the affected object's name
  (e.g. *Delete System* · *Kien's discord*) instead of a raw API path. The object name is
  resolved server-side and never includes config or secrets.

### Fixed
- The Systems bulk-delete now asks for confirmation before removing hosts.

## [1.4.0] — 2026-06-24

### Added
- 1m / 5m / 15m / 1h downsampling ladder with AWS-style long time ranges.
- Uptime-Kuma-style uptime views — gap-filled charts, down history, mini status bars, events feed.

### Changed
- Services split into a two-pane (list + detail) layout.
- Default monitor retries lowered to 1.

### Fixed
- Services table edit/delete buttons no longer clipped behind a long push URL.
- Service-detail breadcrumb order; parent menu stays active on detail pages.

## [1.3.1] — 2026-06-23

### Changed
- Much slimmer agent — static musl on `scratch` (~3 MB).

## [1.3.0] — 2026-06-23

### Added
- Backup / restore (download & upload, S3-compatible).
- Service detail page with uptime history (click a name to open); workspace column.
- Hour-based raw retention plus container rollup tiers.

## [1.2.1] — 2026-06-23

### Added
- Push monitor endpoint returns a JSON ack (optional status / msg / ping).
- Audit log (middleware records mutating API calls) and `/api/about` build metadata.
- Collapsible sidebar (Infrastructure / Services / Alert / Settings) with Audit, About, and a Down filter.

### Fixed
- Push token is server-owned (never sent from the client) and preserved across edits.

## [1.2.0] — 2026-06-23

### Added
- Many monitor kinds: PostgreSQL, MySQL, MongoDB, Redis, RabbitMQ, DNS, TLS-cert-expiry, and Push (passive).
- "Needs attention" triage view with per-workspace alert thresholds.
- System-level RBAC roles and a Members management UI.
- Workspaces management page.

### Changed
- Nav renamed Systems → Infrastructure and Monitors → Services.
- Machine endpoints moved under `/pub` (one Cloudflare Access bypass covers ingest/install/manifest).
- Full-width content on all pages; darker secondary text for WCAG-AA contrast.

## [1.1.0] — 2026-06-22

### Added
- Embedded **Vue SPA** (replaces the original SSR + HTMX UI).
- Interactive uPlot charts — drag-to-zoom, synced cursor, live polling, gap-aware lines.
- Fleet overview, per-core CPU + load average, container and k8s fleet views.
- Teal design identity with an animated logo / favicon.

### Changed
- Filter mini-language with URL state; workspace-aware fleet view.

## [1.0.0] — 2026-06-20

- Initial release: self-hosted server & service monitoring — a push-based host-metrics
  agent, Uptime-Kuma-style service checks, and alerting, with multi-user workspace-scoped
  RBAC and public status pages.

[Unreleased]: https://github.com/The-Last-Devops/last-monitor/compare/v1.7.2...HEAD
[1.7.2]: https://github.com/The-Last-Devops/last-monitor/compare/v1.7.1...v1.7.2
[1.7.1]: https://github.com/The-Last-Devops/last-monitor/compare/v1.7.0...v1.7.1
[1.7.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.5...v1.6.0
[1.5.5]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.4...v1.5.5
[1.5.4]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.3...v1.5.4
[1.5.3]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.2...v1.5.3
[1.5.2]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.1...v1.5.2
[1.5.1]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.3.1...v1.4.0
[1.3.1]: https://github.com/The-Last-Devops/last-monitor/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.2.1...v1.3.0
[1.2.1]: https://github.com/The-Last-Devops/last-monitor/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/The-Last-Devops/last-monitor/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/The-Last-Devops/last-monitor/releases/tag/v1.0.0
