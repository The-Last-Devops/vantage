//! Passkey (WebAuthn) second factor, backed by webauthn-rs. This holds the relying-
//! party `Webauthn` instance plus short-lived in-memory ceremony state (registration
//! and authentication challenges). HTTP endpoints live in `api/passkey.rs`; the login
//! integration is in `auth.rs`. Credentials persist as serde JSON in the
//! `webauthn_credentials` table. RP id / origin come from env (WEBAUTHN_RP_ID /
//! WEBAUTHN_ORIGIN) and MUST match the served domain.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;
use webauthn_rs::prelude::*;

/// How long a started ceremony (challenge) stays valid.
const CEREMONY_TTL: Duration = Duration::from_secs(300);

pub struct PasskeyState {
    pub webauthn: Webauthn,
    reg: Mutex<HashMap<Uuid, (PasskeyRegistration, Instant)>>,
    auth: Mutex<HashMap<Uuid, (PasskeyAuthentication, Instant)>>,
}

impl PasskeyState {
    /// Build from env. Returns `None` (with a warning) if WEBAUTHN_ORIGIN/RP_ID are
    /// invalid — passkey endpoints then report unavailable, but the rest of the hub runs.
    pub fn from_env() -> Option<Self> {
        let rp_id = std::env::var("WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".into());
        // WEBAUTHN_ORIGIN may be a comma-separated list (e.g. dev :5173 + :8080); the
        // first is the primary, the rest are appended as allowed origins.
        let origin_env =
            std::env::var("WEBAUTHN_ORIGIN").unwrap_or_else(|_| "http://localhost:8080".into());
        let origins: Vec<Url> = origin_env
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| match Url::parse(s) {
                Ok(u) => Some(u),
                Err(e) => {
                    tracing::warn!(error = %e, origin = %s, "skipping invalid WEBAUTHN_ORIGIN");
                    None
                }
            })
            .collect();
        let Some((primary, extra)) = origins.split_first() else {
            tracing::warn!("no valid WEBAUTHN_ORIGIN — passkeys disabled");
            return None;
        };
        let builder = match WebauthnBuilder::new(&rp_id, primary) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "WebAuthn builder failed — passkeys disabled");
                return None;
            }
        };
        let builder = extra
            .iter()
            .fold(builder, |b, o| b.append_allowed_origin(o))
            .rp_name("Vantage");
        match builder.build() {
            Ok(webauthn) => {
                tracing::info!(%rp_id, origins = %origin_env, "passkeys enabled");
                Some(Self {
                    webauthn,
                    reg: Mutex::new(HashMap::new()),
                    auth: Mutex::new(HashMap::new()),
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, "WebAuthn build failed — passkeys disabled");
                None
            }
        }
    }

    pub fn put_reg(&self, user: Uuid, st: PasskeyRegistration) {
        let mut m = self.reg.lock().unwrap();
        m.retain(|_, (_, t)| t.elapsed() < CEREMONY_TTL);
        m.insert(user, (st, Instant::now()));
    }
    pub fn take_reg(&self, user: Uuid) -> Option<PasskeyRegistration> {
        let mut m = self.reg.lock().unwrap();
        m.remove(&user)
            .filter(|(_, t)| t.elapsed() < CEREMONY_TTL)
            .map(|(s, _)| s)
    }
    pub fn put_auth(&self, user: Uuid, st: PasskeyAuthentication) {
        let mut m = self.auth.lock().unwrap();
        m.retain(|_, (_, t)| t.elapsed() < CEREMONY_TTL);
        m.insert(user, (st, Instant::now()));
    }
    pub fn take_auth(&self, user: Uuid) -> Option<PasskeyAuthentication> {
        let mut m = self.auth.lock().unwrap();
        m.remove(&user)
            .filter(|(_, t)| t.elapsed() < CEREMONY_TTL)
            .map(|(s, _)| s)
    }
}
