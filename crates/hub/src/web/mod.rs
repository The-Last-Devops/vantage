//! Server-rendered web UI + JSON endpoints feeding it.
//!
//! Minimal skeleton: a dashboard page (HTML + HTMX + uPlot via CDN) and a JSON
//! endpoint listing servers with their latest metric. Real templating (Askama/Maud),
//! auth, and workspaces come next.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::AppState;

mod monitors;
mod systems;

pub use monitors::*;
pub use systems::*;

/// True if the user may view the given server (admin / read-only admin, or a
/// member of its workspace).
pub async fn can_view_system(
    state: &AppState,
    user: &CurrentUser,
    system_id: Uuid,
) -> Result<bool, StatusCode> {
    if user.can_read_all() {
        return Ok(true);
    }
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM systems s \
         JOIN memberships m ON m.workspace_id = s.workspace_id \
         WHERE s.id = $1 AND m.user_id = $2",
    )
    .bind(system_id)
    .bind(user.id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    Ok(row.is_some())
}
#[derive(serde::Deserialize)]
pub struct RangeQuery {
    #[serde(default)]
    pub range: Option<String>,
}
/// Resolves a UI range to which downsampling tier to read and how to display it.
/// Raw is kept ~8h, 1m→2d, 5m→10d, 15m→45d, 1h→365d, so each range reads the
/// finest tier that still covers it (and stays light). Returns
/// (table_suffix, time_column, window_interval, display_bucket).
pub fn chart_tier(
    range: &Option<String>,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match range.as_deref() {
        Some("30m") => ("", "time", "30 minutes", "1 minute"),
        Some("1h") => ("", "time", "1 hour", "1 minute"),
        Some("3h") => ("", "time", "3 hours", "2 minutes"),
        Some("6h") => ("", "time", "6 hours", "5 minutes"),
        Some("12h") => ("_1m", "bucket", "12 hours", "10 minutes"),
        Some("24h") => ("_1m", "bucket", "24 hours", "15 minutes"),
        Some("7d") => ("_5m", "bucket", "7 days", "1 hour"),
        Some("30d") => ("_15m", "bucket", "30 days", "6 hours"),
        Some("90d") => ("_1h", "bucket", "90 days", "1 day"),
        Some("1y") => ("_1h", "bucket", "365 days", "1 day"),
        _ => ("", "time", "1 hour", "1 minute"),
    }
}
pub(crate) fn internal<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "web handler DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}
