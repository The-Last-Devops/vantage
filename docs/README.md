# Documentation

Index of the project's docs. Start here.

## In this folder (`docs/`)

| Doc | What it covers |
|---|---|
| [API.md](API.md) | HTTP API reference — authentication (PAT / session / agent token), endpoints, RBAC, and the embedded **MCP server** (`POST /mcp`). |
| [auth-2fa-passkey.md](auth-2fa-passkey.md) | Two-factor auth — TOTP + passkeys (WebAuthn): how it works and the design. |
| [exposure.md](exposure.md) | Protecting a public hub — put it behind nginx basic-auth / Cloudflare Zero Trust (allow `/pub/*`), and the exposure self-check. |
| [ROADMAP.md](ROADMAP.md) | Planned features not yet built. |
| [SCHEMA_REVIEW.md](SCHEMA_REVIEW.md) | Database table/field naming conventions and the pre-release standardization pass. |

## Elsewhere in the repo

| Doc | What it covers |
|---|---|
| [../README.md](../README.md) | Project overview & quick start. |
| [../CLAUDE.md](../CLAUDE.md) | **Architecture & conventions** — the load-bearing design decisions (two DBs, push model, workspace RBAC, auth paths, MCP) and the engineering rules to follow. Read before changing anything. |
| [../CHANGELOG.md](../CHANGELOG.md) | Per-release changes (the source for GitHub Release notes). |
| [../deploy/README.md](../deploy/README.md) | **Install & deploy** — Docker (compose / published images), Kubernetes (Helm) with prerequisites, parameter tables, and **troubleshooting**, plus the agents. |
| [../frontend/PLAN.md](../frontend/PLAN.md) | The Vue SPA migration plan (historical). |

## Checks (`scripts/`)

Run these before shipping the area they cover; each is idempotent and self-cleaning.

| Script | What it proves |
|---|---|
| `check-fresh-install.sh` | The real sqlx migrator gets a brand-new hub past `migrations applied` (guards the 3.0.0 `_sqlx_migrations` failure). |
| `check-console-errors.sh` | Every main SPA route opens in headless Chrome with **zero** console errors (catches valid-JS-but-broken code that `vite build` accepts). |
| `check-viewer-system-access.sh` | A workspace **viewer** (not just an admin) can read host/cluster endpoints. |
| `check-build-cache.sh` | The Docker layer cache actually reuses the cargo-chef/npm layers across builds. |
| `check-kube-stats.sh` | The Kubernetes stats SQL — including per-**node** grouping — against a throwaway TimescaleDB. |
| `check-config-migrations.sh` | Config migrations apply in order and converge on the current schema. |

## Conventions for adding docs

- Put cross-cutting reference material in `docs/` and **link it from this index**.
- Keep API.md in sync when you add or change a route (see the validation/MCP rules in CLAUDE.md).
- Architecture decisions and engineering rules live in CLAUDE.md, not here.
