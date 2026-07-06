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
authenticates to the host's sshd with the **logged-in user's own** key, requests a
PTY, and bridges to xterm.js in the browser.

- **k8s works here for free:** the daemonset runs `hostNetwork: true`, so the
  agent's `127.0.0.1:22` *is the node's sshd*. No extra privilege needed.
- Auth/authz/audit reuse the host's own SSH + OS user model.
- Requires sshd on the host/node (VMs and most k8s nodes have it).

#### Credential model — per-user keys (load-bearing)
SSH keys are **per user, not per system/shared**. Anyone who can shell into a host
has their **own OS account** on that host (their pubkey in its `authorized_keys`);
the hub only provides the transport and **stores their private key for them**. So:

- **Keys belong to the ACCOUNT, not the server.** Each user keeps a *library* of named
  keys (`ssh_keys`, per user), reusable across every host. A host only stores
  `shell_enabled` + `ssh_port`.
- **At connect the user chooses the auth method:** (1) **password** — the host SSH
  password, typed at connect and never stored; or (2) **key** — pick one key from their
  library, unsealed with their account password. `russh` does `authenticate_password`
  or `authenticate_publickey` accordingly.
- Each key is encrypted under a **key derived from the user's own password** (argon2
  over a dedicated `kdf_salt`, *not* the auth-hash salt). The step-up password unlocks
  it: decrypt in RAM → SSH → wipe.
- **No server master key.** The hub cannot decrypt *anyone's* key without that user
  actively supplying their password — a DB/env leak alone reveals nothing.
- Password reset/forgot ⇒ the user's stored keys become undecryptable; they re-add
  (only their own — there is no shared key to lose).
- Tier 1 still: hub sees the plaintext key in RAM *at use time* (it runs the SSH
  client). True "server never sees the key" is impossible without a browser SSH
  client (WASM) — out of scope.
- Future: derive the KEK from a **WebAuthn PRF** secret (passkey/biometric) instead of
  the password; until then biometrics serve only as a step-up *authorizer*.

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
`owner` of the host's workspace **and** hold a dedicated `can_exec` capability
(system admin bypasses). So "can edit alert rules" ≠ "can shell into prod".
New gate `rbac::require_exec(state, user, workspace_id)` is the single place this
rule lives.

## A hub compromise alone cannot exec

The Tier-1 agent tunnel is now **on by default** (set `ALLOW_SHELL=0` to opt a host out;
Tier-2 host-exec stays a hard opt-in via `ALLOW_HOST_EXEC=1` + privileged). So the tunnel
being open is *not* the security boundary — it's only a byte-pipe to the node's loopback
sshd. The real boundary is **per-user SSH auth**: the hub stores **no usable credential**
(each user's key is sealed under their own password; password auth is typed at connect),
so even a fully-compromised hub with open tunnels to every host cannot authenticate a
shell. To open a console a caller still needs: `shell_enabled` on the system, `require_exec`
(owner + can_exec), a step-up password, and the host's own SSH acceptance.

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
- **Immutable audit:** every session records who / system / workspace / start / end /
  client IP / status **and the full PTY transcript**, append-only, surfaced in an
  Audit UI. No transcript → no shell.
- **No root by default:** Tier 1 runs as the user's own SSH account; Tier 2 (root) is a
  distinct privileged opt-in.
- **Session limits:** idle timeout, max duration, concurrent-session cap, and an
  admin "kill any live session" control.
- **Secrets:** each user's SSH key encrypted at rest (AEAD under their own
  password-derived KEK — no server master key), redacted in every read path, never
  logged, only ever decrypted transiently with the user's step-up password.
- **TLS required** for the browser↔hub and agent↔hub channels.

## Implementation phases

1. **Foundations (safe, no exec):** migrations `0017_exec.sql` (`memberships.can_exec`,
   `systems.shell_enabled`/`ssh_port`, `exec_sessions` + `exec_transcript` append-only),
   `0018` (interim per-(user,system) creds), `0019_ssh_keys.sql` (drop that; account-level
   `ssh_keys` library); `rbac::require_exec`. AEAD + argon2-KEK helper. Hub-only deps.
2. **Agent reverse tunnel (forward-only):** `ALLOW_SHELL`, outbound WS to `/pub/tunnel`,
   stream mux, dial `127.0.0.1:ssh_port`. Agent never execs.
3. **Hub SSH client + PTY bridge:** `russh` through the tunnel, PTY → browser WS
   (`/api/systems/:id/console`), transcript recording.
4. **UI + controls:** xterm.js Console page, step-up modal, live-session list,
   admin kill, Audit page; Add System SSH fields + "enable shell" toggle.
5. **Tier 2 nsenter host-exec** (privileged, opt-in) — separate change, after the above.
