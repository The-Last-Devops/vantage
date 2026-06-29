//! Authentication: password hashing, DB-backed sessions, the `CurrentUser`
//! request extractor, and admin bootstrap.
//!
//! Sessions are opaque random tokens stored in the `sessions` table (revocable),
//! delivered as an httpOnly cookie. Auth is deliberately isolated here so future
//! providers (OAuth/OIDC/LDAP) only need to mint a session, nothing else changes.

use anyhow::Result;
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::{FromRequestParts, OptionalFromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;

pub(crate) const SESSION_COOKIE: &str = "session";
const SESSION_DAYS: i64 = 30;

/// Whether to set the `Secure` attribute on the session cookie. Defaults to
/// `true` so the session token never travels over plaintext HTTP. Local dev
/// runs the hub on http://localhost:8080, where a hard `Secure` would stop
/// login over http — so it can be disabled by setting `INSECURE_COOKIES=1`.
fn secure_cookies() -> bool {
    !matches!(std::env::var("INSECURE_COOKIES").as_deref(), Ok("1"))
}

/// The authenticated user, extracted from the session cookie on each request.
#[derive(Clone, Debug, Serialize)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
    /// System-level read-only ("admin read"): may view every namespace.
    pub read_all: bool,
}

impl CurrentUser {
    /// May this user read across all namespaces? (full admin or read-only admin)
    pub fn can_read_all(&self) -> bool {
        self.is_admin || self.read_all
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Humans authenticate with the session cookie; programmatic callers (scripts,
        // third parties, the MCP server) send `Authorization: Bearer <pat>`.
        let jar = CookieJar::from_headers(&parts.headers);
        if let Some(c) = jar.get(SESSION_COOKIE) {
            if let Some(u) = user_from_session(state, c.value()).await? {
                return Ok(u);
            }
        }
        if let Some(tok) = parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(u) = user_from_pat(state, tok).await? {
                return Ok(u);
            }
        }
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn user_from_session(
    state: &AppState,
    token: &str,
) -> Result<Option<CurrentUser>, StatusCode> {
    let row: Option<(Uuid, String, bool, bool)> = sqlx::query_as(
        "SELECT u.id, u.email, u.is_admin, u.read_all FROM sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE s.token = $1 AND s.expires_at > now()",
    )
    .bind(token)
    .fetch_optional(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "session lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(row.map(|(id, email, is_admin, read_all)| CurrentUser {
        id,
        email,
        is_admin,
        read_all,
    }))
}

/// Hex SHA-256 of a token — what we store/compare for PATs (tokens are
/// high-entropy, so a fast hash is fine; argon2 is only for human passwords).
pub fn token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

async fn user_from_pat(state: &AppState, token: &str) -> Result<Option<CurrentUser>, StatusCode> {
    let row: Option<(Uuid, String, bool, bool, Uuid)> = sqlx::query_as(
        "SELECT u.id, u.email, u.is_admin, u.read_all, p.id FROM api_pats p \
         JOIN users u ON u.id = p.user_id \
         WHERE p.token_hash = $1 AND (p.expires_at IS NULL OR p.expires_at > now())",
    )
    .bind(token_hash(token))
    .fetch_optional(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "pat lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let Some((id, email, is_admin, read_all, pat_id)) = row else {
        return Ok(None);
    };
    // best-effort "last used" stamp; never block auth on it
    let _ = sqlx::query("UPDATE api_pats SET last_used = now() WHERE id = $1")
        .bind(pat_id)
        .execute(&state.config)
        .await;
    Ok(Some(CurrentUser {
        id,
        email,
        is_admin,
        read_all,
    }))
}

/// Optional variant for HTML page handlers: yields `None` (instead of 401) when
/// there's no valid session, so the handler can redirect to /login.
impl OptionalFromRequestParts<AppState> for CurrentUser {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        match <CurrentUser as FromRequestParts<AppState>>::from_request_parts(parts, state).await {
            Ok(user) => Ok(Some(user)),
            Err(StatusCode::UNAUTHORIZED) => Ok(None),
            Err(other) => Err(other),
        }
    }
}

// ---- password hashing -------------------------------------------------------

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash: {e}"))?
        .to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .and_then(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed))
        .is_ok()
}

/// Verify a `password` against the stored hash of `user_id` (for step-up auth, e.g.
/// before opening a shell). Returns `Ok(false)` on a mismatch or unknown user.
pub async fn verify_user_password(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    password: &str,
) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .map(|(h,)| verify_password(password, &h))
        .unwrap_or(false))
}

// ---- handlers ---------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
    /// TOTP code (or a backup code), required only when the account has 2FA enabled.
    #[serde(default)]
    pub totp_code: Option<String>,
    /// A WebAuthn assertion (passkey) — the navigator.credentials.get() result.
    #[serde(default)]
    pub passkey_credential: Option<serde_json::Value>,
}

/// Login response: either `{ twofa_required: true }` (no session minted — the SPA must
/// re-submit with a code) or the [`CurrentUser`] object on success.
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginReq>,
) -> Result<(CookieJar, Json<serde_json::Value>), StatusCode> {
    let user: Option<(Uuid, String, String, bool, bool)> = sqlx::query_as(
        "SELECT id, email, password_hash, is_admin, read_all FROM users WHERE email = $1",
    )
    .bind(&req.email)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;

    let (id, email, password_hash, is_admin, read_all) = user.ok_or(StatusCode::UNAUTHORIZED)?;
    if !verify_password(&req.password, &password_hash) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Second factor (opt-in). If enabled, require a valid TOTP / backup code before a
    // session is minted; signal the SPA to collect one when it's missing.
    let totp_on = crate::api::is_enabled(&state.config, id)
        .await
        .map_err(internal)?;
    let passkey_on = crate::api::has_passkeys(&state.config, id)
        .await
        .map_err(internal)?;
    if totp_on || passkey_on {
        if let Some(cred_json) = req.passkey_credential.clone() {
            // passkey assertion
            let cred = serde_json::from_value(cred_json).map_err(|_| StatusCode::BAD_REQUEST)?;
            if !crate::api::finish_login(&state, id, cred).await? {
                return Err(StatusCode::UNAUTHORIZED);
            }
        } else if let Some(code) = req
            .totp_code
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            // TOTP / backup code
            if !totp_on || !crate::api::verify_login(&state, id, code).await? {
                return Err(StatusCode::UNAUTHORIZED);
            }
        } else {
            // no factor supplied → tell the SPA what's available (+ a passkey challenge)
            let passkey = crate::api::start_login(&state, id).await?;
            return Ok((
                jar,
                Json(serde_json::json!({
                    "twofa_required": true,
                    "totp": totp_on,
                    "passkey": passkey,
                })),
            ));
        }
    }

    // Provision the user's SSH-key master key on first login post-upgrade (we have
    // the plaintext password here). Best-effort — never block login on it.
    if let Err(e) =
        crate::masterkey::provision_if_missing(&state.config, &state.app_secrets, id, &req.password)
            .await
    {
        tracing::warn!(error = %e, user = %email, "master key provisioning on login failed");
    }

    let jar = mint_session(&state, jar, id).await?;
    let me = CurrentUser {
        id,
        email,
        is_admin,
        read_all,
    };
    Ok((jar, Json(serde_json::to_value(me).map_err(internal)?)))
}

pub async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<CookieJar, StatusCode> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(c.value())
            .execute(&state.config)
            .await;
    }
    // Mirror mint_session's attributes so the browser actually clears the cookie
    // (a removal cookie must match path/http_only/same_site/secure to take effect).
    let removal = Cookie::build(SESSION_COOKIE)
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure_cookies())
        .build();
    Ok(jar.remove(removal))
}

pub async fn me(user: CurrentUser) -> Json<CurrentUser> {
    Json(user)
}

// ---- first-run setup --------------------------------------------------------

#[derive(Serialize)]
pub struct SetupStatus {
    /// True when no users exist yet → show the "create admin" wizard.
    pub needs_setup: bool,
}

/// GET /api/setup — public. Tells the SPA whether to show first-run admin creation.
pub async fn setup_status(State(state): State<AppState>) -> Json<SetupStatus> {
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(&state.config)
        .await
        .unwrap_or((0,));
    Json(SetupStatus {
        needs_setup: count.0 == 0,
    })
}

/// POST /api/setup — public, but ONLY succeeds while the users table is empty.
/// Creates the first admin and logs them in (mints a session). Idempotency is by
/// the empty-table guard: once an admin exists this returns 403.
pub async fn setup_create(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginReq>,
) -> Result<(CookieJar, Json<CurrentUser>), StatusCode> {
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    if count.0 != 0 {
        return Err(StatusCode::FORBIDDEN);
    }
    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') || !crate::api::valid_password(&req.password) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hash = hash_password(&req.password).map_err(internal)?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (email, password_hash, is_admin) VALUES ($1, $2, true) RETURNING id",
    )
    .bind(&email)
    .bind(&hash)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    let jar = mint_session(&state, jar, id).await?;
    Ok((
        jar,
        Json(CurrentUser {
            id,
            email,
            is_admin: true,
            read_all: false,
        }),
    ))
}

/// Creates a session row + sets the httpOnly cookie. Shared by login & setup.
async fn mint_session(
    state: &AppState,
    jar: CookieJar,
    user_id: Uuid,
) -> Result<CookieJar, StatusCode> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let expires = chrono::Utc::now() + chrono::Duration::days(SESSION_DAYS);
    sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&token)
        .bind(user_id)
        .bind(expires)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure_cookies())
        .build();
    Ok(jar.add(cookie))
}

// ---- bootstrap --------------------------------------------------------------

/// Creates the initial admin from ADMIN_EMAIL / ADMIN_PASSWORD if it doesn't
/// exist yet. Idempotent; safe to run on every startup.
pub async fn bootstrap_admin(pool: &sqlx::PgPool) -> Result<()> {
    let (email, password) = match (
        std::env::var("ADMIN_EMAIL"),
        std::env::var("ADMIN_PASSWORD"),
    ) {
        (Ok(e), Ok(p)) if !e.is_empty() && !p.is_empty() => (e, p),
        _ => return Ok(()),
    };

    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(pool)
        .await?;
    if exists.is_some() {
        return Ok(());
    }

    // Operator-supplied via env: warn (don't block startup — that could lock them out)
    // if it doesn't meet the password policy applied to UI-set passwords.
    if !crate::api::valid_password(&password) {
        tracing::warn!(
            "ADMIN_PASSWORD is weak (under the 12-char, mixed, non-common policy) — \
             change it after first login"
        );
    }

    let hash = hash_password(&password)?;
    sqlx::query("INSERT INTO users (email, password_hash, is_admin) VALUES ($1, $2, true)")
        .bind(&email)
        .bind(&hash)
        .execute(pool)
        .await?;
    tracing::info!(%email, "bootstrapped admin user");
    Ok(())
}

fn internal<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "auth DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}
