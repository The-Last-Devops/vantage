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

use uuid::Uuid;

use crate::auth::{CurrentUser, SESSION_COOKIE};
use crate::AppState;

/// Best-effort *display name* of the object a mutating request targets, looked up
/// BEFORE the handler runs so a DELETE can still resolve it. Returns names already
/// shown elsewhere in the UI — never config or secrets. `None` for collection
/// creates (no object exists yet) or unknown paths.
async fn object_name(state: &AppState, path: &str) -> Option<String> {
    let segs: Vec<&str> = path
        .trim_start_matches("/api/")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let is_id = |s: &str| Uuid::parse_str(s).is_ok();
    // Operating on one object: `.../{noun}/{id}` or `.../{noun}/{id}/{action}`.
    let (noun, id) = match segs.as_slice() {
        [.., noun, id] if is_id(id) => (*noun, *id),
        [.., noun, id, action] if is_id(id) && matches!(*action, "test" | "run" | "revoke") => {
            (*noun, *id)
        }
        _ => return None, // collection POST etc. — no single existing object
    };
    let uid = Uuid::parse_str(id).ok()?;
    let sql = match noun {
        "channels" => "SELECT name FROM channels WHERE id = $1",
        "monitors" => "SELECT name FROM monitors WHERE id = $1",
        "systems" => "SELECT name FROM systems WHERE id = $1",
        "namespaces" => "SELECT name FROM namespaces WHERE id = $1",
        "users" => "SELECT email FROM users WHERE id = $1",
        // an alert rule has no name of its own — identify it by its target
        "alerts" => {
            "SELECT COALESCE(m.name, s.name) FROM alerts a \
                     LEFT JOIN monitors m ON m.id = a.monitor_id \
                     LEFT JOIN systems s ON s.id = a.system_id WHERE a.id = $1"
        }
        _ => return None,
    };
    sqlx::query_as::<_, (Option<String>,)>(sql)
        .bind(uid)
        .fetch_optional(&state.config)
        .await
        .ok()
        .flatten()
        .and_then(|(n,)| n)
}

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

    // Resolve who + which object BEFORE running the handler, so a DELETE can still
    // name the row it's about to remove.
    let (email, object) = if interesting {
        (
            caller_email(&state, &jar).await,
            object_name(&state, &path).await,
        )
    } else {
        (None, None)
    };
    let res = next.run(req).await;

    // Only record actions tied to a human session; anonymous machine calls are noise.
    if interesting && email.is_some() {
        let _ = sqlx::query(
            "INSERT INTO audit_log (user_email, method, path, status, object_name) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(email)
        .bind(method.as_str())
        .bind(&path)
        .bind(res.status().as_u16() as i32)
        .bind(object)
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
    /// Display name of the affected object (e.g. the channel's name); null when
    /// not resolvable (collection creates, deleted unknown paths).
    pub object_name: Option<String>,
}

/// GET /api/audit — admins read the recent action log (newest first).
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<AuditRow>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(String, Option<String>, String, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    user_email, method, path, status, object_name FROM audit_log \
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
            .map(
                |(at, user_email, method, path, status, object_name)| AuditRow {
                    at,
                    user_email,
                    method,
                    path,
                    status,
                    object_name,
                },
            )
            .collect(),
    ))
}
