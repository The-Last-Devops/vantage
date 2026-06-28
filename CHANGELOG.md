# Changelog

All notable changes to **Vantage** are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each released version's section is used verbatim as the GitHub Release notes
(extracted by `.github/workflows/release.yml`), so keep entries user-facing.

## [Unreleased]

## [2.2.0] — 2026-06-29

### Added
- **New Overview dashboard** — an attention-first landing: open incidents lead, then a
  health KPI strip, with the fleet CPU trend demoted to the bottom.
- **Fleet war-room** (`Fleet`) — every host & service at a glance: a namespace-grouped
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
- **Grant shell access to members from the UI.** In the member editor, each namespace
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
  namespace. (Previously only `push_token` was stripped, so read-only members could
  read stored credentials.)
- **SSRF egress guard on all outbound requests.** Service probes, notification
  webhooks, and the S3 backup endpoint resolve the target and reject loopback,
  link-local / cloud-metadata (`169.254.169.254`), CGNAT, and other reserved
  addresses — including IPv4-mapped forms and redirect hops — before connecting.
  This closes a read-SSRF where an editor could point an HTTP monitor at the cloud
  metadata endpoint and read the response back via the debug view. Private
  (RFC1918/ULA) targets stay allowed so internal monitoring works; set
  `EGRESS_POLICY=strict` to block those too.
- **Agent enrollment keys masked for viewers.** The namespace key list only shows
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
  Alerts, Monitors, Notifications, Members, Namespaces, Events, Audit, API tokens,
  and Data retention no longer flashes a spinner. The spinner appears only on a
  genuine first load or the first time you view a new namespace selection.

### Fixed
- **Cached data can't silently go stale.** Pages re-validate when the browser tab
  regains focus or the network reconnects, and never display a snapshot older than
  60 seconds without refreshing — so a long-open page won't show outdated values.
  Cached data is also cleared on logout.

## [1.7.2] — 2026-06-26

### Changed
- **Alert notifications are now properly formatted** — a title plus Type / Namespace /
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
  is now editable** (re-targets in place). Namespace members are added from a
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
- **Namespace-wide alert rules.** One rule can watch **all services** or **all hosts** in a
  namespace (new hosts/services are covered automatically) — no more editing a rule per target.
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
- **Namespaces redesigned** into a card grid; each namespace has a detail page to manage its
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
  `list_services`, `alerts_firing`, `recent_events` (read, scoped to your namespaces) and
  `run_service_check`, `toggle_alert_rule` (write, require editor of the target's namespace).

## [1.5.5] — 2026-06-25

### Added
- A newly created service is **probed immediately**, so its status (and any alert on
  it) shows at once instead of waiting for the next scheduler cycle.
- Viewing a notify channel lists the **alert rules that use it** (with their namespace
  and enabled state) — click one to jump to the rule.
- Namespace-scoped data now **shows its namespace**: alert rules, the events feed, and
  the services list are each labelled with the namespace they belong to.

### Changed
- **Graceful shutdown** — the hub drains in-flight requests and the agent stops cleanly
  on SIGTERM / Ctrl-C (Docker/k8s can stop them within the grace period).
- Click anywhere on a rule or channel card to open it (action buttons excepted).

### Fixed
- Toggling an alert rule no longer looked like it toggled a *different* rule — the list
  now keeps a stable order instead of re-sorting on every change.
- The Rules list no longer renders under the loader and jumps up when loading finishes.
- **Mobile**: the Services two-pane layout no longer overflows the screen (it stacks),
  and the sidebar's namespace selector + logout are no longer hidden by the browser bar.

## [1.5.4] — 2026-06-25

### Security
- Channel secrets (tokens, passwords, webhook URLs) are masked for anyone who
  can't edit the channel; only editors of the channel's namespace see them.
- Push-monitor tokens are no longer returned to viewers — shown only to editors
  on the monitor detail page (the token is a write credential).
- Credential-bearing request headers (Authorization / Cookie / API-key) are
  redacted in the monitor debug view.

### Added
- **Notify channels are a shared resource**: every namespace can see and use any
  channel in its alert rules; only an editor of a channel's own namespace can
  edit or delete it. New `GET /api/channels`.
- Click a rule or channel card to view it (read-only when you lack edit rights).

### Changed
- Alert **Rules** and **Events** now span all selected namespaces — previously the
  list silently collapsed to a single namespace when "all" or several were picked.

### Fixed
- Mobile: the namespace selector and logout were pushed below the browser toolbar
  in the sidebar (now uses dynamic viewport height).

## [1.5.3] — 2026-06-25

### Added
- **Discord** notify channels can post into a specific **thread** — a new optional
  "Thread ID" field on the Discord channel form.

### Fixed
- **Services** now respects the sidebar namespace filter — the service list, the
  Up/Down/Paused/Total stats, and the Recent-events feed all scope to the selected
  namespace(s), matching Infrastructure (previously Services ignored the filter).

## [1.5.2] — 2026-06-25

### Added
- **Members** redesign — search + role filter, an *Add member* dialog and an edit
  slide-over, per-namespace access shown as named role chips, and a clearer legend
  distinguishing system roles from namespace roles.
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
- Service detail page with uptime history (click a name to open); namespace column.
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
- "Needs attention" triage view with per-namespace alert thresholds.
- System-level RBAC roles and a Members management UI.
- Namespaces management page.

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
- Filter mini-language with URL state; namespace-aware fleet view.

## [1.0.0] — 2026-06-20

- Initial release: self-hosted server & service monitoring — a push-based host-metrics
  agent, Uptime-Kuma-style service checks, and alerting, with multi-user namespace-scoped
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
