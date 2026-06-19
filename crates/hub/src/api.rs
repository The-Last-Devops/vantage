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
    let hash = crate::auth::hash_password(&req.password).map_err(internal)?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (email, password_hash, is_admin) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&req.email)
    .bind(&hash)
    .bind(req.is_admin)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
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
}

/// GET /api/namespaces — namespaces visible to the caller (all for admins).
pub async fn list_namespaces(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<NamespaceRow>>, StatusCode> {
    let rows: Vec<(Uuid, String, Option<String>)> = if user.is_admin {
        sqlx::query_as(
            "SELECT n.id, n.name, m.role::text \
             FROM namespaces n \
             LEFT JOIN memberships m ON m.namespace_id = n.id AND m.user_id = $1 \
             ORDER BY n.name",
        )
        .bind(user.id)
        .fetch_all(&state.config)
        .await
    } else {
        sqlx::query_as(
            "SELECT n.id, n.name, m.role::text \
             FROM namespaces n \
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
            .map(|(id, name, role)| NamespaceRow {
                id,
                name,
                role: role.unwrap_or_else(|| "admin".into()),
            })
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
    }))
}

// ---- members ----------------------------------------------------------------

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

// ---- servers ----------------------------------------------------------------

// ---- enrollment tokens (reusable; servers auto-register) --------------------

#[derive(Serialize)]
pub struct CreatedToken {
    pub id: Uuid,
    pub name: String,
    /// The reusable enrollment token (use as AGENT_TOKEN on any number of hosts).
    pub token: String,
}

#[derive(Deserialize)]
pub struct CreateToken {
    pub name: String,
}

/// POST /api/namespaces/:id/tokens — editors+ mint a reusable enrollment token.
pub async fn create_token(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateToken>,
) -> Result<Json<CreatedToken>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Standardized 32-char token (UUIDv4, hex, ~122 bits of entropy).
    let token = Uuid::new_v4().simple().to_string();
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO enrollment_tokens (namespace_id, name, token) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(ns)
    .bind(&name)
    .bind(&token)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(CreatedToken { id, name, token }))
}

#[derive(Serialize)]
pub struct TokenRow {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub server_count: i64,
}

/// GET /api/namespaces/:id/tokens
pub async fn list_tokens(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<TokenRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(Uuid, String, String, i64)> = sqlx::query_as(
        "SELECT t.id, t.name, t.token, count(s.id) \
         FROM enrollment_tokens t LEFT JOIN servers s ON s.token_id = t.id \
         WHERE t.namespace_id = $1 GROUP BY t.id ORDER BY t.name",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, token, server_count)| TokenRow {
                id,
                name,
                token,
                server_count,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct TokenServers {
    pub token_name: String,
    pub servers: Vec<String>,
}

/// GET /api/tokens/:id/servers — which servers a token enrolled (for delete warning).
pub async fn token_servers(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<TokenServers>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM enrollment_tokens WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let token_name: (String,) = sqlx::query_as("SELECT name FROM enrollment_tokens WHERE id = $1")
        .bind(id)
        .fetch_one(&state.config)
        .await
        .map_err(internal)?;
    let servers: Vec<(String, String)> =
        sqlx::query_as("SELECT name, hostname FROM servers WHERE token_id = $1 ORDER BY hostname")
            .bind(id)
            .fetch_all(&state.config)
            .await
            .map_err(internal)?;
    Ok(Json(TokenServers {
        token_name: token_name.0,
        servers: servers.into_iter().map(|(_, h)| h).collect(),
    }))
}

/// DELETE /api/tokens/:id — removes the token and cascades to every server it enrolled.
pub async fn delete_token(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM enrollment_tokens WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM enrollment_tokens WHERE id = $1")
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
    if !matches!(req.kind.as_str(), "http" | "tcp" | "ping" | "keyword") {
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
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, name, kind FROM notification_channels WHERE namespace_id = $1 ORDER BY name",
    )
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
        "INSERT INTO notification_channels (namespace_id, name, kind, config) \
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
    pub server_id: Option<Uuid>,
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
        "SELECT r.id, r.monitor_id, r.server_id, r.channel_id, r.cooldown_secs, r.enabled \
         FROM alert_rules r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN servers s ON s.id = r.server_id \
         WHERE COALESCE(m.namespace_id, s.namespace_id) = $1",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, monitor_id, server_id, channel_id, cooldown_secs, enabled)| AlertRow {
                    id,
                    monitor_id,
                    server_id,
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
    pub server_id: Option<Uuid>,
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
    if req.monitor_id.is_none() && req.server_id.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // The channel must belong to this namespace.
    let ok: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM notification_channels WHERE id = $1 AND namespace_id = $2")
            .bind(req.channel_id)
            .bind(ns)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;
    if ok.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO alert_rules (monitor_id, server_id, channel_id, condition, cooldown_secs) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(req.monitor_id)
    .bind(req.server_id)
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

/// PATCH /api/servers/:id — rename (cosmetic; namespace is governed by the token).
pub async fn patch_server(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchServer>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM servers WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("UPDATE servers SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(&req.name)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/servers/:id — removes a single server row (it re-registers if its
/// agent is still pushing; use token delete to stop enrollment entirely).
pub async fn delete_server(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM servers WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM servers WHERE id = $1")
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
         interval_secs = COALESCE($4, interval_secs), enabled = COALESCE($5, enabled) WHERE id = $1",
    )
    .bind(id)
    .bind(req.name)
    .bind(req.target)
    .bind(req.interval_secs)
    .bind(req.enabled)
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
        "SELECT namespace_id FROM notification_channels WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM notification_channels WHERE id = $1")
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
        "SELECT COALESCE(m.namespace_id, s.namespace_id) FROM alert_rules r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN servers s ON s.id = r.server_id WHERE r.id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    sqlx::query("DELETE FROM alert_rules WHERE id = $1")
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
