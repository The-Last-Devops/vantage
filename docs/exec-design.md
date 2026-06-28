# Shell / exec — security design

> Status: **design approved, not yet implemented.** This is the most dangerous
> feature in the product (CLAUDE.md principle #1). Build it tier by tier; never
> weaken an invariant below to ship faster.

## Goal

Let an authorized human open an **interactive shell (PTY)** on a monitored host
from the browser console — across VMs, Docker and k8s — without breaking the
push/NAT model and without making a hub compromise equal root on every host.

## The load-bearing constraint

Push model: agents reach **out** to the hub; the hub **never** dials back to an
agent (agents sit behind NAT/firewall). So the hub cannot SSH directly to a host.
The transport must be **agent-mediated** (agent's outbound channel carries the bytes).

## Transport tiers (build in this order)

### Tier 1 — reverse-tunnel to local sshd (PRIMARY, unprivileged)
Agent (when `ALLOW_SHELL=1`) holds one outbound WebSocket to the hub
(`/pub/tunnel`, authed with the existing `x-agent-token`). On session start the
hub opens a muxed stream; the agent **dials `127.0.0.1:<ssh_port>` (default 22)
and pipes bytes** — it never spawns a shell, so the agent gains only "forward to
one local port", not arbitrary exec. The hub runs the SSH client (`russh`),
authenticates to the host's sshd with a stored, encrypted key, requests a PTY,
and bridges to xterm.js in the browser.

- **k8s works here for free:** the daemonset runs `hostNetwork: true`, so the
  agent's `127.0.0.1:22` *is the node's sshd*. No extra privilege needed.
- Auth/authz/audit reuse the host's own SSH + OS user model.
- Hub holds host SSH creds → encrypted at rest, never logged, never in a read path.
  (Future: hub as short-lived SSH CA to avoid long-lived keys.)
- Requires sshd on the host/node (VMs and most k8s nodes have it).

### Tier 2 — agent-mediated `nsenter` host-exec (OPT-IN, privileged) — DEFERRED
For minimal nodes with no sshd. Agent already has `hostPID: true`, so PID 1 is the
host init; `nsenter -t 1 -m -u -i -n -p -- <shell>` gives a host-node shell
("kubectl node-shell" technique). Requires raising the pod to
`securityContext.privileged: true` (or CAP_SYS_ADMIN + SYS_PTRACE) — **full node
root**. Separate hard gate: `ALLOW_HOST_EXEC=1` **and** privileged securityContext,
default off. The agent *does* exec here → biggest blast radius; ship only after
Tier 1 + full audit + step-up are proven.

### Tier 3 — pod/container exec via k8s API — LATER
Debugging an app inside a pod is a different need; proxy the k8s `exec` API with a
scoped ServiceAccount. Not host-exec.

## RBAC — separate `exec` capability (least privilege)

Exec is **not** folded into "edit config". A user may open a shell only if they are
`owner` of the host's namespace **and** hold a dedicated `can_exec` capability
(system admin bypasses). So "can edit alert rules" ≠ "can shell into prod".
New gate `rbac::require_exec(state, user, namespace_id)` is the single place this
rule lives.

## Two-sided opt-in (a hub compromise alone cannot exec)

Shell is **off by default** and must be enabled on **both** sides:
1. **Agent side:** deployed with `ALLOW_SHELL=1` (Tier 1) / `ALLOW_HOST_EXEC=1` (Tier 2).
2. **Hub side:** the system's config has `shell_enabled = true` + SSH connection set.

If an attacker owns only the hub, agents not deployed with the flag refuse the tunnel.

## Privilege escalation (sudo)

Tier 1 is a real PTY over SSH, so `sudo` works whenever the configured SSH login
user is in the host's sudoers — interactive password prompt included. Escalation is
governed by the **host's own sudoers**, not bypassed by the hub: least privilege, the
host stays in control. Log in as a normal sudo-capable user (not root) per the
"no root by default" invariant and escalate on demand; the host's auth/sudo logs
record it too. Tier 2 (nsenter) is already root, so no sudo step.

**Transcript must not capture typed secrets.** Recording the input stream would also
record a sudo (or any) password the user types. Mitigation: record the **output**
stream for audit; for input, detect terminal echo-off (PTY `ECHO` cleared) and mask
those bytes (store a `‹masked›` marker, not the keystrokes).

## Non-negotiable invariants

- **Step-up auth** before opening a shell (re-enter password / 2FA), like sudo.
- **Immutable audit:** every session records who / system / namespace / start / end /
  client IP / status **and the full PTY transcript**, append-only, surfaced in an
  Audit UI. No transcript → no shell.
- **No root by default:** Tier 1 runs as the configured SSH user; Tier 2 (root) is a
  distinct privileged opt-in.
- **Session limits:** idle timeout, max duration, concurrent-session cap, and an
  admin "kill any live session" control.
- **Secrets:** SSH keys encrypted at rest (AEAD, hub master key from env), redacted in
  every read path, never logged.
- **TLS required** for the browser↔hub and agent↔hub channels.

## Implementation phases

1. **Foundations (safe, no exec):** migration `0017_exec.sql` — `memberships.can_exec`,
   `systems` SSH columns + `shell_enabled`, `exec_sessions` + `exec_transcript`
   (append-only) tables; `rbac::require_exec`; AEAD secret helper. Hub-only deps.
2. **Agent reverse tunnel (forward-only):** `ALLOW_SHELL`, outbound WS to `/pub/tunnel`,
   stream mux, dial `127.0.0.1:ssh_port`. Agent never execs.
3. **Hub SSH client + PTY bridge:** `russh` through the tunnel, PTY → browser WS
   (`/api/systems/:id/console`), transcript recording.
4. **UI + controls:** xterm.js Console page, step-up modal, live-session list,
   admin kill, Audit page; Add System SSH fields + "enable shell" toggle.
5. **Tier 2 nsenter host-exec** (privileged, opt-in) — separate change, after the above.
