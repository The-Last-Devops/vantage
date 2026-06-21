-- Config DB: users, sessions, namespaces, RBAC, API keys, systems, monitors,
-- channels, alerts, status pages. Plain PostgreSQL (no TimescaleDB here).
-- Pre-release: this is the single consolidated schema (no incremental history).

CREATE EXTENSION IF NOT EXISTS pgcrypto; -- gen_random_uuid()

CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email         TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,            -- argon2
    is_admin      BOOLEAN NOT NULL DEFAULT false,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Opaque, revocable login sessions (no JWT, so logout/revoke work immediately).
CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,            -- session cookie value (distinct from api_keys)
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);

-- k8s-style namespace: a single DNS-label name identifies it.
CREATE TABLE namespaces (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TYPE ns_role AS ENUM ('owner', 'editor', 'viewer');

CREATE TABLE memberships (
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    namespace_id UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    role         ns_role NOT NULL,
    PRIMARY KEY (user_id, namespace_id)
);

-- API keys: a reusable secret presented by agents (and reusable elsewhere later).
-- One key can enroll MANY systems (e.g. a k8s DaemonSet).
CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key          TEXT NOT NULL UNIQUE,      -- the secret sent in the x-api-key header
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A monitored system: a node, a docker host, or a k8s node. Identified by
-- (key_id, hostname); auto-registers on first metrics push.
CREATE TABLE systems (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id  UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    key_id        UUID NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    hostname      TEXT NOT NULL,
    kind          TEXT NOT NULL DEFAULT 'node',  -- node | docker | k8s
    cluster       TEXT,                          -- k8s cluster name (NULL otherwise)
    enabled       BOOLEAN NOT NULL DEFAULT true,
    last_seen     TIMESTAMPTZ,
    kernel        TEXT,
    cpu_model     TEXT,
    cpu_cores     INTEGER,
    agent_version TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX systems_key_host ON systems (key_id, hostname);
CREATE INDEX idx_systems_namespace ON systems(namespace_id);
CREATE INDEX idx_systems_kind ON systems(kind);

-- A service check (Uptime-Kuma style).
CREATE TYPE monitor_kind AS ENUM ('http', 'tcp', 'ping', 'keyword');

CREATE TABLE monitors (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id  UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          monitor_kind NOT NULL,
    target        TEXT NOT NULL,
    interval_secs INTEGER NOT NULL DEFAULT 60,
    config        JSONB NOT NULL DEFAULT '{}'::jsonb,
    enabled       BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_monitors_namespace ON monitors(namespace_id);

CREATE TABLE channels (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL,             -- telegram | webhook | email
    config       JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE alerts (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    monitor_id    UUID REFERENCES monitors(id) ON DELETE CASCADE,
    system_id     UUID REFERENCES systems(id) ON DELETE CASCADE,
    channel_id    UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    condition     JSONB NOT NULL DEFAULT '{}'::jsonb,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    enabled       BOOLEAN NOT NULL DEFAULT true,
    CHECK (monitor_id IS NOT NULL OR system_id IS NOT NULL)
);

-- Per-alert state so the engine fires once on transition + honours cooldown.
CREATE TABLE alert_state (
    alert_id      UUID PRIMARY KEY REFERENCES alerts(id) ON DELETE CASCADE,
    firing        BOOLEAN NOT NULL DEFAULT false,
    last_changed  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_notified TIMESTAMPTZ
);

CREATE TABLE status_pages (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    namespace_id UUID NOT NULL REFERENCES namespaces(id) ON DELETE CASCADE,
    slug         TEXT NOT NULL UNIQUE,
    title        TEXT NOT NULL,
    config       JSONB NOT NULL DEFAULT '{}'::jsonb,
    is_public    BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
