//! Workspace-scoped authorization. All permission checks funnel through
//! [`require_role`] so new roles or rules are changed in exactly one place.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::AppState;

/// Roles ordered by privilege. `Viewer < Editor < Owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer = 0,
    Editor = 1,
    Owner = 2,
}

impl Role {
    pub fn from_db_str(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Owner => "owner",
        }
    }
}

/// Returns the user's effective role in a workspace, or `None` if not a member.
///
/// System admins are `Owner` everywhere. `read_all` is a **floor, not a ceiling**: it
/// grants `Viewer` in every workspace, and an explicit membership can still raise that
/// on the workspaces where the user is meant to do real work. (It used to return
/// `Viewer` and stop, which silently discarded an `editor`/`owner` membership — a
/// read-only admin could be made an owner of one workspace in the UI and get nothing.)
pub async fn role_in(
    state: &AppState,
    user: &CurrentUser,
    workspace_id: Uuid,
) -> Result<Option<Role>, StatusCode> {
    if user.is_admin {
        return Ok(Some(Role::Owner));
    }
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT role::text FROM memberships WHERE user_id = $1 AND workspace_id = $2",
    )
    .bind(user.id)
    .bind(workspace_id)
    .fetch_optional(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "role lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let membership = row.and_then(|(r,)| Role::from_db_str(&r));
    if user.read_all {
        // Viewer everywhere, or better where a membership says so.
        return Ok(Some(membership.unwrap_or(Role::Viewer).max(Role::Viewer)));
    }
    Ok(membership)
}

/// Authorizes that `user` has at least `min` role in `workspace_id`.
/// Maps to 403 when under-privileged, 404-style 403 when not a member at all.
pub async fn require_role(
    state: &AppState,
    user: &CurrentUser,
    workspace_id: Uuid,
    min: Role,
) -> Result<Role, StatusCode> {
    match role_in(state, user, workspace_id).await? {
        Some(role) if role >= min => Ok(role),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

/// Authorizes shell/exec on a workspace's host. The user must be `Owner` of the
/// workspace **and** hold the dedicated `can_exec` capability — exec is kept
/// separate from "edit config" (see docs/exec-design.md). System admins bypass. Being
/// `read_all` grants nothing here — exec needs a real `owner` membership carrying
/// `can_exec`, which an owner has to hand out deliberately. 403 otherwise. The single
/// place the exec rule lives (mirrors require_role).
pub async fn require_exec(
    state: &AppState,
    user: &CurrentUser,
    workspace_id: Uuid,
) -> Result<(), StatusCode> {
    if user.is_admin {
        return Ok(());
    }
    // read_all by itself is not exec-capable: this reads memberships only, so the
    // capability has to be granted explicitly on this workspace.
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT role::text, can_exec FROM memberships WHERE user_id = $1 AND workspace_id = $2",
    )
    .bind(user.id)
    .bind(workspace_id)
    .fetch_optional(&state.config)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "exec authz lookup");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match row {
        Some((role, true)) if Role::from_db_str(&role) == Some(Role::Owner) => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ordering() {
        assert!(Role::Owner > Role::Editor);
        assert!(Role::Editor > Role::Viewer);
        // an editor satisfies a viewer requirement but not an owner one
        assert!(Role::Editor >= Role::Viewer);
        assert!(Role::Editor < Role::Owner);
    }

    #[test]
    fn db_roundtrip() {
        for r in [Role::Viewer, Role::Editor, Role::Owner] {
            assert_eq!(Role::from_db_str(r.as_db()), Some(r));
        }
        assert_eq!(Role::from_db_str("nope"), None);
    }

    #[test]
    fn require_min_semantics() {
        // `require_role(.., need)` passes when the user's role >= need.
        assert!(Role::Owner >= Role::Editor); // owner can do editor work
        assert!(Role::Owner >= Role::Owner);
        assert!(Role::Viewer < Role::Editor); // viewer can't do editor work
        assert!(Role::Editor < Role::Owner); // editor can't manage members
    }

    #[test]
    fn from_db_str_is_case_sensitive() {
        assert_eq!(Role::from_db_str("Owner"), None);
        assert_eq!(Role::from_db_str("EDITOR"), None);
        assert_eq!(Role::from_db_str(""), None);
    }

    #[test]
    fn as_db_strings_are_stable() {
        assert_eq!(Role::Viewer.as_db(), "viewer");
        assert_eq!(Role::Editor.as_db(), "editor");
        assert_eq!(Role::Owner.as_db(), "owner");
    }
}
