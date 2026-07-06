//! Service-monitor JSON endpoints feeding the SPA.
//!
//! Split by concern, each re-exported below so `web::list_monitors`,
//! `web::monitor_detail`, `web::monitor_heartbeats`, `web::monitor_events`, and
//! `web::recent_events` paths stay unchanged:
//! - [`list`] — the monitors list with each monitor's latest beat + sparkline.
//! - [`detail`] — one monitor with status, current-run duration and uptime %.
//! - [`events`] — heartbeat history + status-transition feeds (per-monitor and
//!   global).
//!
//! [`load_monitor`] (authorize + load a monitor) and [`hb_range`] (the
//! heartbeat-history range table) are shared by detail and events, so they live
//! here.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::web::internal;
use crate::AppState;

mod detail;
mod events;
mod list;

pub use detail::monitor_detail;
pub use events::{monitor_events, monitor_heartbeats, recent_events};
pub use list::list_monitors;

/// True if the user may view the given monitor; returns its (workspace, name,
/// kind, target, interval, enabled, config) when so.
#[allow(clippy::type_complexity)]
pub(super) async fn load_monitor(
    state: &AppState,
    user: &CurrentUser,
    id: Uuid,
) -> Result<(String, String, String, String, i32, bool, serde_json::Value), StatusCode> {
    let row: Option<(
        String,
        String,
        String,
        String,
        i32,
        bool,
        sqlx::types::Json<serde_json::Value>,
    )> = sqlx::query_as(
        "SELECT n.name, m.name, m.kind::text, m.target, m.interval_secs, m.enabled, m.config \
             FROM monitors m JOIN workspaces n ON n.id = m.workspace_id \
             WHERE m.id = $1 AND ($2 OR m.workspace_id IN ( \
                SELECT workspace_id FROM memberships WHERE user_id = $3))",
    )
    .bind(id)
    .bind(user.can_read_all())
    .bind(user.id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    let (workspace, name, kind, target, interval_secs, enabled, config) =
        row.ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        workspace,
        name,
        kind,
        target,
        interval_secs,
        enabled,
        config.0,
    ))
}

const SECRET_MASK: &str = "••••••";

/// True for header names that carry credentials and must be masked in read paths.
/// Mirrors the sensitive-header set used when probing (see `probe::checks`).
fn secret_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "api-key"
            | "x-auth-token"
            | "x-token"
            | "token"
    )
}

/// Redact every credential a monitor's `config` can hold, in place:
/// drop `push_token`; mask the value of any credential-bearing `headers` entry;
/// and mask `auth.password` / `auth.token` / `password` when present & non-empty.
/// Non-secret keys are left intact. Apply this in any read path shown to callers
/// who can't edit the monitor.
pub(super) fn redact_monitor_config(config: &mut serde_json::Value) {
    let Some(o) = config.as_object_mut() else {
        return;
    };
    o.remove("push_token");

    if let Some(headers) = o.get_mut("headers").and_then(|v| v.as_object_mut()) {
        for (k, v) in headers.iter_mut() {
            if secret_header(k) {
                *v = serde_json::Value::String(SECRET_MASK.into());
            }
        }
    }

    if let Some(auth) = o.get_mut("auth").and_then(|v| v.as_object_mut()) {
        for key in ["password", "token"] {
            if let Some(v) = auth.get_mut(key) {
                if v.as_str().is_some_and(|s| !s.is_empty()) {
                    *v = serde_json::Value::String(SECRET_MASK.into());
                }
            }
        }
    }

    if let Some(v) = o.get_mut("password") {
        if v.as_str().is_some_and(|s| !s.is_empty()) {
            *v = serde_json::Value::String(SECRET_MASK.into());
        }
    }
}

/// Mask the password embedded in a connection-string `target`
/// (`scheme://user:pass@host…` → `scheme://user:***@host…`). Targets without
/// userinfo/password (plain host:port, bare URLs, etc.) are returned unchanged.
/// The DB monitor kinds (postgres/mysql/mongodb) put the password here.
pub(super) fn mask_target(target: &str) -> String {
    // Locate the authority: the part after `scheme://`, up to the first `/?#`.
    let Some(scheme_end) = target.find("://") else {
        return target.to_string();
    };
    let auth_start = scheme_end + 3;
    let auth_end = target[auth_start..]
        .find(['/', '?', '#'])
        .map(|i| auth_start + i)
        .unwrap_or(target.len());
    let authority = &target[auth_start..auth_end];
    // userinfo is everything before the last '@' in the authority; a password
    // exists only if that userinfo contains a ':'.
    let Some(at) = authority.rfind('@') else {
        return target.to_string();
    };
    let userinfo = &authority[..at];
    let Some(colon) = userinfo.find(':') else {
        return target.to_string();
    };
    let pass_start = auth_start + colon + 1;
    let pass_end = auth_start + at;
    if pass_start >= pass_end {
        return target.to_string(); // empty password
    }
    format!("{}***{}", &target[..pass_start], &target[pass_end..])
}

/// (interval, bucket) for the heartbeat history chart — supports up to 30 days.
pub(super) fn hb_range(range: &Option<String>) -> (&'static str, &'static str) {
    match range.as_deref() {
        Some("1h") => ("1 hour", "1 minute"),
        Some("6h") => ("6 hours", "5 minutes"),
        Some("7d") => ("7 days", "1 hour"),
        Some("30d") => ("30 days", "6 hours"),
        Some("90d") => ("90 days", "1 day"),
        Some("1y") => ("365 days", "1 day"),
        _ => ("24 hours", "15 minutes"),
    }
}
