//! Audit trail. A middleware logs every mutating API call (who/what/when/result)
//! into `audit_log`; admins read it via GET /api/audit.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Serialize;

use crate::auth::{CurrentUser, SESSION_COOKIE};
use crate::AppState;

/// Resolve the caller's email from the session cookie (best-effort; None if absent).
async fn caller_email(state: &AppState, jar: &CookieJar) -> Option<String> {
    let token = jar.get(SESSION_COOKIE)?.value().to_owned();
    sqlx::query_as::<_, (String,)>(
        "SELECT u.email FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token = $1 AND s.expires_at > now()",
    )
    .bind(&token)
    .fetch_optional(&state.config)
    .await
    .ok()
    .flatten()
    .map(|(e,)| e)
}

/// Middleware: record mutating /api calls after they run.
pub async fn record(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let jar = CookieJar::from_headers(req.headers());
    let mutating = matches!(method.as_str(), "POST" | "PATCH" | "PUT" | "DELETE");
    // This is a *user* action log. Skip login (would log before a session exists),
    // agent ingest (machine traffic, no session, fires constantly), and non-API paths.
    let interesting = mutating
        && path.starts_with("/api/")
        && path != "/api/auth/login"
        && !path.starts_with("/api/ingest");

    let email = if interesting {
        caller_email(&state, &jar).await
    } else {
        None
    };
    let res = next.run(req).await;

    // Only record actions tied to a human session; anonymous machine calls are noise.
    if interesting && email.is_some() {
        let _ = sqlx::query(
            "INSERT INTO audit_log (user_email, method, path, status) VALUES ($1, $2, $3, $4)",
        )
        .bind(email)
        .bind(method.as_str())
        .bind(&path)
        .bind(res.status().as_u16() as i32)
        .execute(&state.config)
        .await;
    }
    res
}

#[derive(Serialize)]
pub struct AuditRow {
    pub at: String,
    pub user_email: Option<String>,
    pub method: String,
    pub path: String,
    pub status: i32,
}

/// GET /api/audit — admins read the recent action log (newest first).
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<AuditRow>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(String, Option<String>, String, String, i32)> = sqlx::query_as(
        "SELECT to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                user_email, method, path, status FROM audit_log \
         ORDER BY at DESC LIMIT 500",
    )
    .fetch_all(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "audit list");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(
        rows.into_iter()
            .map(|(at, user_email, method, path, status)| AuditRow {
                at,
                user_email,
                method,
                path,
                status,
            })
            .collect(),
    ))
}
