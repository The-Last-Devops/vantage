-- Shell/exec foundations — see docs/exec-design.md.
-- Phase 1: schema only. No exec path exists yet; these columns/tables are inert
-- until the tunnel + SSH bridge land. Everything defaults to OFF.

-- RBAC: a dedicated exec capability, separate from the namespace role.
-- A user may open a shell only if owner of the system's namespace AND can_exec
-- (system admin bypasses). "Edit config" never implies "shell into prod".
ALTER TABLE memberships
    ADD COLUMN can_exec BOOLEAN NOT NULL DEFAULT false;

-- Per-system shell config. shell_enabled is the hub side of the two-sided opt-in
-- (the agent side is the ALLOW_SHELL env). SSH key is encrypted at rest (AEAD);
-- it is a credential — redact in every read path, never log.
ALTER TABLE systems
    ADD COLUMN shell_enabled        BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN ssh_host             TEXT,                       -- defaults to 127.0.0.1 via the agent tunnel
    ADD COLUMN ssh_port             INTEGER NOT NULL DEFAULT 22,
    ADD COLUMN ssh_user             TEXT,
    ADD COLUMN ssh_key_enc          BYTEA,                      -- AEAD ciphertext (nonce||ct); NULL = unset
    ADD COLUMN ssh_key_fingerprint  TEXT;                       -- shown in UI; safe (public)

-- Immutable session audit. Rows survive system/user deletion (snapshot the names,
-- SET NULL the FKs) so the audit trail can't be erased by removing a system.
CREATE TABLE exec_sessions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    system_id     UUID REFERENCES systems(id)    ON DELETE SET NULL,
    system_name   TEXT NOT NULL,                  -- snapshot
    namespace_id  UUID REFERENCES namespaces(id) ON DELETE SET NULL,
    user_id       UUID REFERENCES users(id)      ON DELETE SET NULL,
    user_email    TEXT NOT NULL,                  -- snapshot
    transport     TEXT NOT NULL,                  -- 'ssh' (Tier 1) | 'nsenter' (Tier 2)
    ssh_user      TEXT,                           -- OS user the shell ran as
    client_ip     TEXT,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at      TIMESTAMPTZ,
    status        TEXT NOT NULL DEFAULT 'active', -- active | closed | killed | error
    error         TEXT
);
CREATE INDEX idx_exec_sessions_namespace ON exec_sessions(namespace_id);
CREATE INDEX idx_exec_sessions_started   ON exec_sessions(started_at DESC);
CREATE INDEX idx_exec_sessions_status    ON exec_sessions(status);

-- Append-only transcript: ordered chunks of input (keystrokes) and output.
-- No shell without a recorded transcript (docs/exec-design.md invariant).
CREATE TABLE exec_transcript (
    session_id  UUID NOT NULL REFERENCES exec_sessions(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    stream      TEXT NOT NULL,   -- 'in' (keystrokes) | 'out' (terminal output)
    data        BYTEA NOT NULL,
    PRIMARY KEY (session_id, seq)
);
