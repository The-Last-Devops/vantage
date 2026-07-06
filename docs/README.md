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
| [../deploy/README.md](../deploy/README.md) | **Install & deploy** — Docker (compose / published images), Kubernetes (Helm), and agents. |
| [../frontend/PLAN.md](../frontend/PLAN.md) | The Vue SPA migration plan (historical). |

## Conventions for adding docs

- Put cross-cutting reference material in `docs/` and **link it from this index**.
- Keep API.md in sync when you add or change a route (see the validation/MCP rules in CLAUDE.md).
- Architecture decisions and engineering rules live in CLAUDE.md, not here.
