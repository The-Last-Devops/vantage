//! Passkey (WebAuthn) second factor, backed by webauthn-rs. This holds short-lived
//! in-memory ceremony state (registration and authentication challenges) plus the
//! relying-party config. The `Webauthn` relying party is built per request: by default
//! the RP id / origin are derived from the request's `Origin`/`Host` header so passkeys
//! work on whatever domain serves the hub with no extra config. Setting WEBAUTHN_RP_ID
//! and/or WEBAUTHN_ORIGIN pins them instead (e.g. when the hub is reachable on several
//! hosts and you want one canonical RP). HTTP endpoints live in `api/passkey.rs`; the
//! login integration is in `auth.rs`. Credentials persist as serde JSON in the
//! `webauthn_credentials` table.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;
use webauthn_rs::prelude::*;

/// How long a started ceremony (challenge) stays valid.
const CEREMONY_TTL: Duration = Duration::from_secs(300);

pub struct PasskeyState {
    /// Pinned RP from env (rp_id, origins). When set, every ceremony uses it and the
    /// request `Origin` is ignored. When `None`, the RP is derived per request from the
    /// browser-supplied `Origin` (falling back to `Host`) — the zero-config default.
    pinned: Option<(String, Vec<Url>)>,
    reg: Mutex<HashMap<Uuid, (PasskeyRegistration, Instant)>>,
    auth: Mutex<HashMap<Uuid, (PasskeyAuthentication, Instant)>>,
}

impl PasskeyState {
    /// Always available. Reads WEBAUTHN_RP_ID / WEBAUTHN_ORIGIN: if either is set it
    /// pins the RP from env (legacy behaviour); if both are unset the RP is derived per
    /// request from the `Origin` header (no env needed).
    pub fn from_env() -> Option<Self> {
        // Treat empty/whitespace as unset (compose passes `${WEBAUTHN_RP_ID:-}` through
        // as "" when the operator didn't set it — that must still mean "auto-derive").
        let nonempty = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let rp_id_env = nonempty("WEBAUTHN_RP_ID");
        let origin_env = nonempty("WEBAUTHN_ORIGIN");
        let pinned = if rp_id_env.is_none() && origin_env.is_none() {
            tracing::info!("passkeys enabled (RP derived from request Origin — no WEBAUTHN_* env)");
            None
        } else {
            // WEBAUTHN_ORIGIN may be a comma-separated list (e.g. dev :5173 + :8080).
            let rp_id = rp_id_env.unwrap_or_else(|| "localhost".into());
            let origin_str = origin_env.unwrap_or_else(|| "http://localhost:8080".into());
            let origins: Vec<Url> = origin_str
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
            if origins.is_empty() {
                tracing::warn!("no valid WEBAUTHN_ORIGIN — falling back to per-request Origin");
                None
            } else {
                tracing::info!(%rp_id, origins = %origin_str, "passkeys enabled (pinned RP from env)");
                Some((rp_id, origins))
            }
        };
        Some(Self {
            pinned,
            reg: Mutex::new(HashMap::new()),
            auth: Mutex::new(HashMap::new()),
        })
    }

    /// Build the relying party for one ceremony. Uses the pinned env RP when configured,
    /// else derives it from the request `origin` header (falling back to `host` with an
    /// assumed https scheme). Returns `None` if neither yields a usable origin.
    pub fn build(&self, origin: Option<&str>, host: Option<&str>) -> Option<Webauthn> {
        let (rp_id, origins) = match &self.pinned {
            Some((id, o)) => (id.clone(), o.clone()),
            None => {
                let url = origin
                    .and_then(|o| Url::parse(o).ok())
                    .or_else(|| host.and_then(|h| Url::parse(&format!("https://{h}")).ok()))?;
                (url.host_str()?.to_string(), vec![url])
            }
        };
        let (primary, extra) = origins.split_first()?;
        let mut builder = WebauthnBuilder::new(&rp_id, primary).ok()?;
        for o in extra {
            builder = builder.append_allowed_origin(o);
        }
        builder.rp_name("Vantage").build().ok()
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
