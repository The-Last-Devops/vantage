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
mod pats;
mod servers;
mod users;

pub use alerting::*;
pub use data::*;
pub use keys::*;
pub use monitors::*;
pub use namespaces::*;
pub use pats::*;
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

#[cfg(test)]
mod name_tests {
    use super::valid_name;

    #[test]
    fn accepts_normal_names() {
        assert!(valid_name("ops-alerts", 64));
        assert!(valid_name("Kien's Discord", 64));
        assert!(valid_name("máy chủ Hà Nội", 64)); // unicode is fine
        assert!(valid_name("a", 1));
    }

    #[test]
    fn rejects_blank_or_whitespace_only() {
        assert!(!valid_name("", 64));
        assert!(!valid_name("   ", 64));
        assert!(!valid_name("\t\n", 64));
    }

    #[test]
    fn rejects_too_long_after_trim() {
        assert!(!valid_name(&"x".repeat(65), 64));
        assert!(valid_name(&format!("  {}  ", "x".repeat(64)), 64)); // trims to 64
    }

    #[test]
    fn rejects_control_characters() {
        assert!(!valid_name("bad\u{0007}name", 64)); // bell
        assert!(!valid_name("line\nbreak", 64));
    }

    #[test]
    fn counts_unicode_scalars_not_bytes() {
        // "é" is 2 bytes but 1 char — a 1-char limit must accept it.
        assert!(valid_name("é", 1));
    }
}
