//! Management API: namespaces, members, servers (with agent-token issuance),
//! and monitors. Every namespaced route authorizes via [`rbac::require_role`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::rbac::{self, Role};
use crate::AppState;

mod alerting;
mod data;
mod keys;
mod monitors;
mod namespaces;
mod servers;
mod users;

pub use alerting::*;
pub use data::*;
pub use keys::*;
pub use monitors::*;
pub use namespaces::*;
pub use servers::*;
pub use users::*;

/// A user-facing display name (channel / monitor / system / status-page title…):
/// non-empty after trimming, at most `max` characters, and free of control
/// characters. Spaces, punctuation and unicode are fine — this rejects only
/// blank or junk input. Slugs/identifiers use the stricter [`valid_ns_name`].
pub fn valid_name(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= max && !t.chars().any(char::is_control)
}

fn internal<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "api DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}
#[derive(Serialize)]
pub struct About {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub build_date: &'static str,
}
/// GET /api/about — build metadata for the About page.
pub async fn about(_user: CurrentUser) -> Json<About> {
    Json(About {
        version: env!("CARGO_PKG_VERSION"),
        git_sha: env!("GIT_SHA"),
        build_date: env!("BUILD_DATE"),
    })
}

// ---- users (admin-only provisioning) ---------------------------------------
async fn ns_of(state: &AppState, sql: &str, id: Uuid) -> Result<Uuid, StatusCode> {
    sqlx::query_as::<_, (Uuid,)>(sql)
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .map_err(internal)?
        .map(|(ns,)| ns)
        .ok_or(StatusCode::NOT_FOUND)
}
