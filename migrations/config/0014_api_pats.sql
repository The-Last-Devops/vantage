-- Personal access tokens for programmatic API + MCP access. Distinct from agent
-- enrollment keys (api_keys) and human login sessions. A PAT acts AS its user, so
-- it inherits that user's RBAC — scope a token by issuing it to a service-account
-- user with limited namespace membership. Only the SHA-256 hash is stored.
CREATE TABLE api_pats (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,   -- sha256(hex) of the full token
    prefix     TEXT NOT NULL,          -- leading chars for display, e.g. "lm_pat_ab12…"
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used  TIMESTAMPTZ,
    expires_at TIMESTAMPTZ             -- NULL = never expires
);
CREATE INDEX idx_api_pats_user ON api_pats(user_id);
