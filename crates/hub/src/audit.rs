//! Audit trail. A middleware logs every mutating API call (who/what/when/result)
//! into `audit_log`; admins read it via GET /api/audit.

use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};

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
        "workspaces" => "SELECT name FROM workspaces WHERE id = $1",
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

#[derive(Deserialize)]
pub struct AuditQuery {
    /// Free-text match against user / endpoint / object.
    pub q: Option<String>,
    /// Exact HTTP method: POST | PATCH | PUT | DELETE.
    pub method: Option<String>,
    /// Result class: ok (<300) | client (4xx) | server (5xx).
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct AuditPage {
    pub rows: Vec<AuditRow>,
    /// Total rows matching the filters (for pagination), not just this page.
    pub total: i64,
    /// Current retention setting (days); null = kept forever.
    pub retention_days: Option<i32>,
}

/// Delete audit rows older than the configured retention window. No-op when
/// retention is unset (keep forever). Best-effort; errors are swallowed.
pub async fn prune(state: &AppState) {
    if let Some(d) = crate::settings::get_opt::<i32>(&state.config, "audit_retention_days").await {
        if d > 0 {
            let _ = sqlx::query(&format!(
                "DELETE FROM audit_log WHERE at < now() - interval '{d} days'"
            ))
            .execute(&state.config)
            .await;
        }
    }
}

/// GET /api/audit — admins read the action log (newest first), filtered + paginated.
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(qp): Query<AuditQuery>,
) -> Result<Json<AuditPage>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    // Pruning on read keeps the table bounded without a dedicated background loop.
    prune(&state).await;

    let q = qp.q.filter(|s| !s.trim().is_empty());
    let method = qp.method.filter(|s| !s.trim().is_empty());
    let status = qp.status.filter(|s| !s.trim().is_empty());
    let limit = qp.limit.unwrap_or(100).clamp(1, 500);
    let offset = qp.offset.unwrap_or(0).max(0);

    // Every filter is bound and made optional in SQL (`$n IS NULL OR ...`) so the
    // bind order is fixed regardless of which filters are present.
    let where_sql = "WHERE ($1::text IS NULL OR (user_email ILIKE '%'||$1||'%' \
                     OR path ILIKE '%'||$1||'%' OR object_name ILIKE '%'||$1||'%')) \
                     AND ($2::text IS NULL OR method = $2) \
                     AND ($3::text IS NULL \
                          OR ($3 = 'ok' AND status < 300) \
                          OR ($3 = 'client' AND status >= 400 AND status < 500) \
                          OR ($3 = 'server' AND status >= 500))";

    let (total,): (i64,) = sqlx::query_as(&format!("SELECT count(*) FROM audit_log {where_sql}"))
        .bind(&q)
        .bind(&method)
        .bind(&status)
        .fetch_one(&state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "audit count");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let rows: Vec<(String, Option<String>, String, String, i32, Option<String>)> =
        sqlx::query_as(&format!(
            "SELECT to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                user_email, method, path, status, object_name FROM audit_log {where_sql} \
         ORDER BY at DESC LIMIT $4 OFFSET $5",
        ))
        .bind(&q)
        .bind(&method)
        .bind(&status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "audit list");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let retention_days =
        crate::settings::get_opt::<i32>(&state.config, "audit_retention_days").await;

    Ok(Json(AuditPage {
        rows: rows
            .into_iter()
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
        total,
        retention_days,
    }))
}

#[derive(Deserialize)]
pub struct RetentionReq {
    /// Days to keep; null or 0 = keep forever.
    pub days: Option<i32>,
}

/// PUT /api/admin/audit/retention — set how long the audit log is kept (admins).
pub async fn set_retention(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<RetentionReq>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let days = req.days.filter(|d| *d > 0); // 0/negative → keep forever (null)
    crate::settings::set(&state.config, "audit_retention_days", &days)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "audit retention");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    prune(&state).await;
    Ok(StatusCode::NO_CONTENT)
}
