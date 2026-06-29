-- Passkeys (WebAuthn) as a second factor (docs/auth-2fa-passkey.md, model B).
-- A user may register one or more platform/roaming authenticators; at sign-in,
-- after the password, they assert with the passkey. Verified with the pure-Rust
-- `webauthn_rp` crate (RustCrypto + ring, no OpenSSL).
--
-- `cred_data` stores the credential's serialized static state (public key, etc.)
-- via webauthn_rp's binary Encode; `sign_count` is the rolling signature counter
-- (clone-detection). RP id / origin come from env (WEBAUTHN_RP_ID/WEBAUTHN_ORIGIN).

CREATE TABLE webauthn_credentials (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cred_id     BYTEA NOT NULL UNIQUE,            -- the raw credential id
    cred_data   BYTEA NOT NULL,                   -- serialized RegisteredCredential static state
    sign_count  BIGINT NOT NULL DEFAULT 0,        -- rolling signature counter
    name        TEXT NOT NULL,                    -- user's label ("MacBook Touch ID", "YubiKey")
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used   TIMESTAMPTZ
);
CREATE INDEX idx_webauthn_user ON webauthn_credentials(user_id);
