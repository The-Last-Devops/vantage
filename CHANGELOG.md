# Changelog

All notable changes to **Last Monitor** are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each released version's section is used verbatim as the GitHub Release notes
(extracted by `.github/workflows/release.yml`), so keep entries user-facing.

## [Unreleased]

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

[Unreleased]: https://github.com/The-Last-Devops/last-monitor/compare/v1.5.4...HEAD
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
