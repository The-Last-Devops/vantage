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

const SESSION_COOKIE: &str = "session";
const SESSION_DAYS: i64 = 30;

/// The authenticated user, extracted from the session cookie on each request.
#[derive(Clone, Debug, Serialize)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|c| c.value().to_owned())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let row: Option<(Uuid, String, bool)> = sqlx::query_as(
            "SELECT u.id, u.email, u.is_admin FROM sessions s \
             JOIN users u ON u.id = s.user_id \
             WHERE s.token = $1 AND s.expires_at > now()",
        )
        .bind(&token)
        .fetch_optional(&state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "session lookup");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        match row {
            Some((id, email, is_admin)) => Ok(CurrentUser {
                id,
                email,
                is_admin,
            }),
            None => Err(StatusCode::UNAUTHORIZED),
        }
    }
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

// ---- handlers ---------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginReq>,
) -> Result<(CookieJar, Json<CurrentUser>), StatusCode> {
    let user: Option<(Uuid, String, String, bool)> =
        sqlx::query_as("SELECT id, email, password_hash, is_admin FROM users WHERE email = $1")
            .bind(&req.email)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;

    let (id, email, password_hash, is_admin) = user.ok_or(StatusCode::UNAUTHORIZED)?;
    if !verify_password(&req.password, &password_hash) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let jar = mint_session(&state, jar, id).await?;
    Ok((
        jar,
        Json(CurrentUser {
            id,
            email,
            is_admin,
        }),
    ))
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
    Ok(jar.remove(Cookie::from(SESSION_COOKIE)))
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
    if email.is_empty() || !email.contains('@') || req.password.len() < 6 {
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
