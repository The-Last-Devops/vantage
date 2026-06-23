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

fn internal<E: std::fmt::Display>(e: E) -> StatusCode {
    tracing::error!(error = %e, "api DB error");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ---- users (admin-only provisioning) ---------------------------------------

#[derive(Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub read_all: bool,
}

/// POST /api/users — admins provision accounts (no open registration).
pub async fn create_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateUser>,
) -> Result<Json<Uuid>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let email = req.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') || req.password.len() < 6 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let hash = crate::auth::hash_password(&req.password).map_err(internal)?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (email, password_hash, is_admin, read_all) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&email)
    .bind(&hash)
    .bind(req.is_admin)
    .bind(req.read_all)
    .fetch_one(&state.config)
    .await
    .map_err(|_| StatusCode::CONFLICT)?; // unique email violation → 409
    Ok(Json(id))
}

#[derive(Serialize)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
    pub read_all: bool,
    pub created_at: String,
    pub namespaces: i64,
}

/// GET /api/users — admins list all accounts with their system role + ns count.
pub async fn list_users(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<UserRow>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(Uuid, String, bool, bool, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.email, u.is_admin, u.read_all, u.created_at::text, \
            (SELECT count(*) FROM memberships m WHERE m.user_id = u.id) \
         FROM users u ORDER BY u.email",
    )
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, email, is_admin, read_all, created_at, namespaces)| UserRow {
                    id,
                    email,
                    is_admin,
                    read_all,
                    created_at,
                    namespaces,
                },
            )
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct PatchUser {
    pub is_admin: Option<bool>,
    pub read_all: Option<bool>,
    pub password: Option<String>,
}

/// PATCH /api/users/:id — admins change system role / reset password.
pub async fn patch_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchUser>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    // don't let an admin remove their own admin rights (avoids locking everyone out)
    if id == user.id && req.is_admin == Some(false) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(is_admin) = req.is_admin {
        sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
            .bind(is_admin)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    if let Some(read_all) = req.read_all {
        sqlx::query("UPDATE users SET read_all = $1 WHERE id = $2")
            .bind(read_all)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    if let Some(password) = req.password {
        if password.len() < 6 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let hash = crate::auth::hash_password(&password).map_err(internal)?;
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(&hash)
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct UserMembership {
    pub namespace_id: Uuid,
    pub namespace: String,
    pub role: String,
}

/// GET /api/users/:id/memberships — admins list a user's per-namespace roles
/// (for the user editor). Namespaces the user isn't in are simply absent.
pub async fn user_memberships(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<UserMembership>>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT n.id, n.name, m.role::text FROM memberships m \
         JOIN namespaces n ON n.id = m.namespace_id WHERE m.user_id = $1 ORDER BY n.name",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(namespace_id, namespace, role)| UserMembership {
                namespace_id,
                namespace,
                role,
            })
            .collect(),
    ))
}

/// DELETE /api/users/:id — admins remove an account (not their own).
pub async fn delete_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if id == user.id {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- data management (admin) ------------------------------------------------

/// GET /api/admin/data — DB size, per-table size/rows, retention tiers.
pub async fn data_stats(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<crate::data_admin::DataStats>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(crate::data_admin::stats(&state.data).await))
}

#[derive(Deserialize)]
pub struct SetRetention {
    pub table: String,
    pub days: i64,
}

/// POST /api/admin/retention — change a tier's retention window.
pub async fn set_retention(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<SetRetention>,
) -> Result<StatusCode, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    crate::data_admin::set_retention(&state.data, &req.table, req.days)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- namespaces -------------------------------------------------------------

#[derive(Serialize)]
pub struct NamespaceRow {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub system_count: i64,
    pub member_count: i64,
}

/// GET /api/namespaces — namespaces visible to the caller (all for admins),
/// each with its system and member counts for the management view.
pub async fn list_namespaces(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<NamespaceRow>>, StatusCode> {
    let counts = "(SELECT count(*) FROM systems s WHERE s.namespace_id = n.id), \
                  (SELECT count(*) FROM memberships mm WHERE mm.namespace_id = n.id)";
    let rows: Vec<(Uuid, String, Option<String>, i64, i64)> = if user.can_read_all() {
        sqlx::query_as(&format!(
            "SELECT n.id, n.name, m.role::text, {counts} \
             FROM namespaces n \
             LEFT JOIN memberships m ON m.namespace_id = n.id AND m.user_id = $1 \
             ORDER BY n.name",
        ))
        .bind(user.id)
        .fetch_all(&state.config)
        .await
    } else {
        sqlx::query_as(&format!(
            "SELECT n.id, n.name, m.role::text, {counts} \
             FROM namespaces n \
             JOIN memberships m ON m.namespace_id = n.id \
             WHERE m.user_id = $1 ORDER BY n.name",
        ))
        .bind(user.id)
        .fetch_all(&state.config)
        .await
    }
    .map_err(internal)?;

    Ok(Json(
        rows.into_iter()
            .map(
                |(id, name, role, system_count, member_count)| NamespaceRow {
                    id,
                    name,
                    role: role.unwrap_or_else(|| "admin".into()),
                    system_count,
                    member_count,
                },
            )
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateNamespace {
    pub name: String,
}

/// Validates a k8s-style namespace name: a DNS label, max 63 chars.
pub fn valid_ns_name(name: &str) -> bool {
    let n = name.len();
    if n == 0 || n > 63 {
        return false;
    }
    let bytes = name.as_bytes();
    let edge_ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !edge_ok(bytes[0]) || !edge_ok(bytes[n - 1]) {
        return false;
    }
    name.bytes()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::valid_ns_name;

    #[test]
    fn namespace_names() {
        assert!(valid_ns_name("default"));
        assert!(valid_ns_name("team-a"));
        assert!(valid_ns_name("prod1"));
        assert!(!valid_ns_name("")); // empty
        assert!(!valid_ns_name("Team-A")); // uppercase
        assert!(!valid_ns_name("-lead")); // leading hyphen
        assert!(!valid_ns_name("trail-")); // trailing hyphen
        assert!(!valid_ns_name("a b")); // space
        assert!(!valid_ns_name(&"x".repeat(64))); // too long
    }
}

/// POST /api/namespaces — any authenticated user may create one; creator becomes owner.
pub async fn create_namespace(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<CreateNamespace>,
) -> Result<Json<NamespaceRow>, StatusCode> {
    if !valid_ns_name(&req.name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut tx = state.config.begin().await.map_err(internal)?;
    let (id,): (Uuid,) = sqlx::query_as("INSERT INTO namespaces (name) VALUES ($1) RETURNING id")
        .bind(&req.name)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("INSERT INTO memberships (user_id, namespace_id, role) VALUES ($1, $2, 'owner')")
        .bind(user.id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    Ok(Json(NamespaceRow {
        id,
        name: req.name,
        role: "owner".into(),
        system_count: 0,
        member_count: 1,
    }))
}

/// DELETE /api/namespaces/:id — owners (and admins) only. Refuses to delete the
/// 'default' namespace, or any namespace that still has systems attached
/// (avoids cascading away live hosts by accident).
pub async fn delete_namespace(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    rbac::require_role(&state, &user, id, Role::Owner).await?;

    let (name,): (String,) = sqlx::query_as("SELECT name FROM namespaces WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if name == "default" {
        return Err(StatusCode::FORBIDDEN);
    }

    let (systems,): (i64,) = sqlx::query_as("SELECT count(*) FROM systems WHERE namespace_id = $1")
        .bind(id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    if systems > 0 {
        return Err(StatusCode::CONFLICT);
    }

    sqlx::query("DELETE FROM namespaces WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- alert thresholds (per-namespace, for the "Needs attention" view) -------

/// Warn/crit % thresholds per resource. Defaults: warn 80, crit 90.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Thresholds {
    pub cpu_warn: f64,
    pub cpu_crit: f64,
    pub mem_warn: f64,
    pub mem_crit: f64,
    pub disk_warn: f64,
    pub disk_crit: f64,
    /// Disk I/O utilization (busiest disk % busy).
    #[serde(default = "default_dutil_warn")]
    pub dutil_warn: f64,
    #[serde(default = "default_dutil_crit")]
    pub dutil_crit: f64,
}
fn default_dutil_warn() -> f64 {
    80.0
}
fn default_dutil_crit() -> f64 {
    95.0
}
impl Default for Thresholds {
    fn default() -> Self {
        Self {
            cpu_warn: 80.0,
            cpu_crit: 90.0,
            mem_warn: 80.0,
            mem_crit: 90.0,
            disk_warn: 80.0,
            disk_crit: 90.0,
            dutil_warn: 80.0,
            dutil_crit: 95.0,
        }
    }
}

#[derive(Serialize)]
pub struct NsThresholds {
    pub namespace: String,
    #[serde(flatten)]
    pub t: Thresholds,
}

/// GET /api/thresholds — effective thresholds for every namespace the caller can
/// see (stored override merged onto the defaults). The fleet UI maps these by
/// namespace name to flag abnormal hosts.
pub async fn list_thresholds(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<NsThresholds>>, StatusCode> {
    let rows: Vec<(String, Option<Value>)> = if user.can_read_all() {
        sqlx::query_as("SELECT name, thresholds FROM namespaces ORDER BY name")
            .fetch_all(&state.config)
            .await
    } else {
        sqlx::query_as(
            "SELECT n.name, n.thresholds FROM namespaces n \
             JOIN memberships m ON m.namespace_id = n.id \
             WHERE m.user_id = $1 ORDER BY n.name",
        )
        .bind(user.id)
        .fetch_all(&state.config)
        .await
    }
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(namespace, v)| NsThresholds {
                namespace,
                t: v.and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default(),
            })
            .collect(),
    ))
}

/// PUT /api/namespaces/:id/thresholds — editors+ set the namespace's thresholds.
pub async fn set_thresholds(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(t): Json<Thresholds>,
) -> Result<StatusCode, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    for (w, c) in [
        (t.cpu_warn, t.cpu_crit),
        (t.mem_warn, t.mem_crit),
        (t.disk_warn, t.disk_crit),
        (t.dutil_warn, t.dutil_crit),
    ] {
        if !(0.0..=100.0).contains(&w) || !(0.0..=100.0).contains(&c) || w > c {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let v = serde_json::to_value(t).map_err(internal)?;
    sqlx::query("UPDATE namespaces SET thresholds = $1 WHERE id = $2")
        .bind(v)
        .bind(ns)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- members ----------------------------------------------------------------

#[derive(Serialize)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub email: String,
    pub role: String,
}

/// GET /api/namespaces/:id/members — owners (and admins) list namespace members.
pub async fn list_members(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<MemberRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Owner).await?;
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.email, m.role::text FROM memberships m \
         JOIN users u ON u.id = m.user_id WHERE m.namespace_id = $1 ORDER BY u.email",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(user_id, email, role)| MemberRow {
                user_id,
                email,
                role,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct AddMember {
    pub email: String,
    pub role: String, // viewer | editor | owner
}

/// POST /api/namespaces/:id/members — owners (and admins) manage membership.
pub async fn add_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<AddMember>,
) -> Result<StatusCode, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Owner).await?;
    let role = Role::from_db_str(&req.role).ok_or(StatusCode::BAD_REQUEST)?;

    let (target,): (Uuid,) = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&req.email)
        .fetch_optional(&state.config)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    sqlx::query(
        "INSERT INTO memberships (user_id, namespace_id, role) VALUES ($1, $2, $3::ns_role) \
         ON CONFLICT (user_id, namespace_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(target)
    .bind(ns)
    .bind(role.as_db())
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- API keys (reusable; systems auto-register) -----------------------------

#[derive(Serialize)]
pub struct CreatedKey {
    pub id: Uuid,
    pub name: String,
    /// The reusable API key (sent as x-api-key on any number of hosts).
    pub key: String,
}

#[derive(Deserialize)]
pub struct CreateKey {
    pub name: String,
}

/// POST /api/namespaces/:id/keys — editors+ mint a reusable API key.
pub async fn create_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateKey>,
) -> Result<Json<CreatedKey>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(StatusCode::BAD_REQUEST);
    }
    // 32-char key (UUIDv4 hex, ~122 bits of entropy).
    let key = Uuid::new_v4().simple().to_string();
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO api_keys (namespace_id, name, key) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(ns)
    .bind(&name)
    .bind(&key)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(CreatedKey { id, name, key }))
}

#[derive(Serialize)]
pub struct KeyRow {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub system_count: i64,
}

/// GET /api/namespaces/:id/keys
pub async fn list_keys(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<KeyRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT k.id, k.name, k.key, count(s.id) \
         FROM api_keys k LEFT JOIN systems s ON s.key_id = k.id \
         WHERE k.namespace_id = $1 GROUP BY k.id ORDER BY k.name",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, key, system_count)| KeyRow {
                id,
                name,
                key,
                system_count,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct KeySystems {
    pub key_name: String,
    pub systems: Vec<String>,
}

/// GET /api/keys/:id/systems — which systems a key enrolled (for delete warning).
pub async fn key_systems(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<KeySystems>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM api_keys WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let key_name: (String,) = sqlx::query_as("SELECT name FROM api_keys WHERE id = $1")
        .bind(id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    let systems: Vec<(String, String)> =
        sqlx::query_as("SELECT name, hostname FROM systems WHERE key_id = $1 ORDER BY hostname")
            .bind(id)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    Ok(Json(KeySystems {
        key_name: key_name.0,
        systems: systems.into_iter().map(|(_, h)| h).collect(),
    }))
}

/// DELETE /api/keys/:id — removes the key and cascades to every system it enrolled.
pub async fn delete_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM api_keys WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- monitors ---------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateMonitor {
    pub name: String,
    pub kind: String, // http | tcp | ping | keyword
    pub target: String,
    #[serde(default)]
    pub interval_secs: Option<i32>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/namespaces/:id/monitors — editors+ add a service check.
pub async fn create_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateMonitor>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    if !matches!(
        req.kind.as_str(),
        "http" | "tcp" | "ping" | "keyword" | "postgres" | "redis" | "dns" | "rabbitmq"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO monitors (namespace_id, name, kind, target, interval_secs, config) \
         VALUES ($1, $2, $3::monitor_kind, $4, $5, $6) RETURNING id",
    )
    .bind(ns)
    .bind(&req.name)
    .bind(&req.kind)
    .bind(&req.target)
    .bind(req.interval_secs.unwrap_or(60).max(1))
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;

    Ok(Json(id))
}

/// GET /api/monitors/:id/debug — last ok + last err request/response detail.
pub async fn monitor_debug(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(String, Value, String)> =
        sqlx::query_as("SELECT outcome, detail, at::text FROM monitor_debug WHERE monitor_id = $1")
            .bind(id)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    let (mut ok, mut err) = (Value::Null, Value::Null);
    for (outcome, detail, at) in rows {
        let entry = serde_json::json!({ "detail": detail, "at": at });
        if outcome == "ok" {
            ok = entry;
        } else {
            err = entry;
        }
    }
    Ok(Json(serde_json::json!({ "ok": ok, "err": err })))
}

// ---- notification channels --------------------------------------------------

#[derive(Serialize)]
pub struct ChannelRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
}

/// GET /api/namespaces/:id/channels
pub async fn list_channels(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<ChannelRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, kind FROM channels WHERE namespace_id = $1 ORDER BY name")
            .bind(ns)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind)| ChannelRow { id, name, kind })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub kind: String, // webhook | telegram | email
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/namespaces/:id/channels — editors+ add a notification channel.
pub async fn create_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateChannel>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    if !matches!(req.kind.as_str(), "webhook" | "telegram" | "email") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO channels (namespace_id, name, kind, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ns)
    .bind(&req.name)
    .bind(&req.kind)
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}

// ---- alert rules ------------------------------------------------------------

#[derive(Serialize)]
pub struct AlertRow {
    pub id: Uuid,
    pub monitor_id: Option<Uuid>,
    pub system_id: Option<Uuid>,
    pub channel_id: Uuid,
    pub cooldown_secs: i32,
    pub enabled: bool,
}

/// GET /api/namespaces/:id/alerts — rules whose target lives in this namespace.
pub async fn list_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<AlertRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(Uuid, Option<Uuid>, Option<Uuid>, Uuid, i32, bool)> = sqlx::query_as(
        "SELECT r.id, r.monitor_id, r.system_id, r.channel_id, r.cooldown_secs, r.enabled \
         FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         WHERE COALESCE(m.namespace_id, s.namespace_id) = $1",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, monitor_id, system_id, channel_id, cooldown_secs, enabled)| AlertRow {
                    id,
                    monitor_id,
                    system_id,
                    channel_id,
                    cooldown_secs,
                    enabled,
                },
            )
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateAlert {
    #[serde(default)]
    pub monitor_id: Option<Uuid>,
    #[serde(default)]
    pub system_id: Option<Uuid>,
    pub channel_id: Uuid,
    #[serde(default)]
    pub condition: Option<Value>,
    #[serde(default)]
    pub cooldown_secs: Option<i32>,
}

/// POST /api/namespaces/:id/alerts — editors+ create an alert rule.
pub async fn create_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateAlert>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    if req.monitor_id.is_none() && req.system_id.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // The channel must belong to this namespace.
    let ok: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM channels WHERE id = $1 AND namespace_id = $2")
            .bind(req.channel_id)
            .bind(ns)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;
    if ok.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO alerts (monitor_id, system_id, channel_id, condition, cooldown_secs) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(req.monitor_id)
    .bind(req.system_id)
    .bind(req.channel_id)
    .bind(sqlx::types::Json(
        req.condition.unwrap_or_else(|| serde_json::json!({})),
    ))
    .bind(req.cooldown_secs.unwrap_or(300).max(0))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}

// ---- edit / delete ----------------------------------------------------------

async fn ns_of(state: &AppState, sql: &str, id: Uuid) -> Result<Uuid, StatusCode> {
    sqlx::query_as::<_, (Uuid,)>(sql)
        .bind(id)
        .fetch_optional(&state.config)
        .await
        .map_err(internal)?
        .map(|(ns,)| ns)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
pub struct PatchServer {
    pub name: String,
}

/// PATCH /api/systems/:id — rename (cosmetic; namespace is governed by the token).
pub async fn patch_system(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchServer>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("UPDATE systems SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(&req.name)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/systems/:id — removes a single server row (it re-registers if its
/// agent is still pushing; use token delete to stop enrollment entirely).
pub async fn delete_system(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM systems WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PatchMonitor {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub interval_secs: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// PATCH /api/monitors/:id — edit fields / toggle enabled.
pub async fn patch_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchMonitor>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query(
        "UPDATE monitors SET name = COALESCE($2, name), target = COALESCE($3, target), \
         interval_secs = COALESCE($4, interval_secs), enabled = COALESCE($5, enabled), \
         config = COALESCE($6, config) WHERE id = $1",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.target)
    .bind(req.interval_secs)
    .bind(req.enabled)
    .bind(req.config.map(sqlx::types::Json))
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/monitors/:id
pub async fn delete_monitor(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM monitors WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/channels/:id
pub async fn delete_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM channels WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/status-pages/:id
pub async fn delete_status_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM status_pages WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM status_pages WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/alerts/:id (namespace resolved via the rule's monitor/server).
pub async fn delete_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT COALESCE(m.namespace_id, s.namespace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM alerts WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/namespaces/:id/members/:user_id (owners only).
pub async fn delete_member(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((ns, target)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Owner).await?;
    sqlx::query("DELETE FROM memberships WHERE namespace_id = $1 AND user_id = $2")
        .bind(ns)
        .bind(target)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- status pages -----------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateStatusPage {
    pub slug: String,
    pub title: String,
    /// Optional {"monitor_ids":[...]}; empty => all monitors in the namespace.
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/namespaces/:id/status-pages — editors+ publish a public status page.
pub async fn create_status_page(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateStatusPage>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO status_pages (namespace_id, slug, title, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ns)
    .bind(&req.slug)
    .bind(&req.title)
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}
