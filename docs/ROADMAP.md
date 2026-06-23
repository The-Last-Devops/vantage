# Roadmap

Planned features not yet built. See CLAUDE.md for the architecture they must fit.

## Backup / restore (download/upload + S3-compatible)

Goal: export the **config DB** (users, namespaces, RBAC, API keys, servers, monitors,
alerts, channels, thresholds, status pages) as a portable snapshot and restore it
elsewhere. The **data DB** (metrics/heartbeats) is large, regenerable telemetry and is
out of scope for backup.

**Design:**
- **Logical JSON snapshot** (not `pg_dump`) so it stays single-binary and version-tolerant:
  serialize all config tables → one JSON document; restore upserts in FK order.
- **Download / upload** from the Settings UI (admin only).
- **S3 / S3-compatible**: store endpoint + bucket + region + access/secret key (config DB);
  "Back up to S3 now", list snapshots, restore from one. Later: scheduled backups.

**Must ship with:** admin-only; restore is destructive → require explicit confirm; never
log S3 secrets; redact password hashes are kept (so users still work) but treated as secret.

**Scope:** medium — export/import endpoints, an S3 client, a settings store, a Settings page.

## Web SSH / terminal into hosts

Goal: open a shell on a monitored host from the dashboard.

**Constraint — keep the push model.** The hub never dials agents (they sit behind
NAT). So the shell must ride a connection the agent opens *outbound*.

**Design:**
- Agent opens a persistent **WebSocket** to the hub (same direction as metric push).
- UI "Terminal" → hub sends an "open shell" message over that reverse tunnel →
  agent spawns a **PTY** on the host and pipes stdin/stdout back.
- Browser uses **xterm.js**; flow: browser ⇄ hub (WebSocket) ⇄ agent ⇄ PTY.

**This is a remote-code-execution channel — must ship with:**
- Per-agent opt-in (a flag/env; default OFF).
- Authorization: only `owner`/`admin` on the host's namespace (see RBAC).
- **Audit log** of every session (who, host, when; ideally command/IO capture).
- Restrict the shell user; consider per-session approval.

**Scope:** large — agent PTY + reverse WS, hub WS multiplexing, frontend terminal,
RBAC + audit. Multi-phase.

## Adaptive report interval (realtime only while viewed)

Pipework exists (`IngestAck.next_interval_secs`, agent honours it). Hub currently
returns `0` (no change), so agents always push at their default 60s.

**Design:** hub tracks "host recently viewed" (set when the browser polls
`/api/systems/{id}/metrics` or `/api/fleet`); the ingest handler returns
`next_interval_secs=2` when the reporting host was viewed in the last ~10s, else
`60`. No one watching → 60s (light); someone opens a host → it ramps to realtime
within one push, then back down. Small change, hub-only.
