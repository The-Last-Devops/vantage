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
mod exec;
mod keys;
mod monitors;
mod namespaces;
mod passkey;
mod pats;
mod servers;
mod twofa;
mod users;

pub use alerting::*;
pub use data::*;
pub use exec::*;
pub use keys::*;
pub use monitors::*;
pub use namespaces::*;
pub use passkey::*;
pub use pats::*;
pub use servers::*;
pub use twofa::*;
pub use users::*;

/// A user-facing display name (channel / monitor / system / status-page title…):
/// non-empty after trimming, at most `max` characters, and free of control
/// characters. Spaces, punctuation and unicode are fine — this rejects only
/// blank or junk input. Slugs/identifiers use the stricter [`valid_ns_name`].
pub fn valid_name(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() <= max && !t.chars().any(char::is_control)
}

/// Password policy — long and high-difficulty. Mirror this rule in the Vue form
/// (`lib/password.js`) for instant feedback; the API stays the source of truth.
///
/// Rules: 12–128 chars; at least 3 of {lowercase, uppercase, digit, symbol}; and not
/// an obviously weak/common password. Strength matters doubly now that a user's
/// password derives the KEK that encrypts their stored SSH keys (docs/exec-design.md),
/// so a weak password directly weakens that encryption. Existing logins are
/// unaffected — this gates only setting a *new* password.
pub fn valid_password(s: &str) -> bool {
    let len = s.chars().count();
    if !(12..=128).contains(&len) {
        return false;
    }
    let classes = [
        s.chars().any(|c| c.is_ascii_lowercase()),
        s.chars().any(|c| c.is_ascii_uppercase()),
        s.chars().any(|c| c.is_ascii_digit()),
        s.chars()
            .any(|c| !c.is_alphanumeric() && !c.is_whitespace()),
    ]
    .into_iter()
    .filter(|&b| b)
    .count();
    if classes < 3 {
        return false;
    }
    // Reject obvious/common passwords and our own product words (case-insensitive).
    let low = s.to_lowercase();
    const WEAK: &[&str] = &[
        "password", "passw0rd", "qwerty", "123456", "111111", "000000", "letmein", "welcome",
        "iloveyou", "abc123", "admin", "vantage", "monitor",
    ];
    if WEAK.iter().any(|w| low.contains(w)) {
        return false;
    }
    true
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
    use super::{valid_name, valid_password};

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

    #[test]
    fn password_accepts_strong() {
        assert!(valid_password("Tr0ub4dour&3xtra")); // 4 classes, long
        assert!(valid_password("correct-Horse9Battery")); // lower/upper/digit/symbol
        assert!(valid_password("Xy9$kLmn0pQr")); // exactly 12, 4 classes
    }

    #[test]
    fn password_rejects_too_short_or_too_long() {
        assert!(!valid_password("Sh0rt!")); // < 12
        assert!(!valid_password("Xy9$kLmn0pQ")); // 11 chars
        assert!(!valid_password(&format!("Aa1!{}", "x".repeat(130)))); // > 128
    }

    #[test]
    fn password_rejects_low_diversity() {
        assert!(!valid_password("alllowercaseletters")); // 1 class
        assert!(!valid_password("onlylettersANDCAPS")); // 2 classes
        assert!(!valid_password("123456789012345")); // 1 class (digits)
    }

    #[test]
    fn password_rejects_common_and_product_words() {
        assert!(!valid_password("MyPassword123!")); // contains "password"
        assert!(!valid_password("Vantage2026!!")); // product word
        assert!(!valid_password("Qwerty12345!")); // common
    }
}
