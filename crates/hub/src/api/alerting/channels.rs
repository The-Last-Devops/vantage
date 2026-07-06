//! Notify-channel API: list/create/patch/delete channels, the provider-type
//! manifest, the channels-in-use view, and config tests (saved + unsaved).

use super::*;

#[derive(Serialize)]
pub struct ChannelRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub config: Value,
}

/// GET /api/workspaces/:id/channels
pub async fn list_channels(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
) -> Result<Json<Vec<ChannelRow>>, StatusCode> {
    let role = rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    // Secrets (tokens/passwords/webhook URLs) go only to those who can edit the
    // channel; viewers get them masked. Editors need the real config to edit.
    let can_see_secrets = role >= Role::Editor;
    let rows: Vec<(Uuid, String, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT id, name, kind, config FROM channels WHERE workspace_id = $1 ORDER BY name",
    )
    .bind(ws)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind, config)| {
                let config = if can_see_secrets {
                    config.0
                } else {
                    crate::notify::redact_secrets(&kind, &config.0)
                };
                ChannelRow {
                    id,
                    name,
                    kind,
                    config,
                }
            })
            .collect(),
    ))
}

#[derive(Serialize)]
pub struct GlobalChannelRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub workspace: String,
    pub workspace_id: Uuid,
    /// Whether the caller may edit/delete this channel (editor+ of its workspace).
    pub can_edit: bool,
    pub config: Value,
}

/// GET /api/channels — every channel across all workspaces. Channels are a shared
/// resource: anyone signed in may view them and attach them to alerts. Only an
/// editor of the channel's OWN workspace may edit/delete it, so secrets are shown
/// only to those editors; everyone else gets them masked.
pub async fn list_all_channels(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<GlobalChannelRow>>, StatusCode> {
    // Workspaces the caller can edit (system admins edit everywhere).
    let editable: std::collections::HashSet<Uuid> = if user.is_admin {
        std::collections::HashSet::new()
    } else {
        sqlx::query_as::<_, (Uuid,)>(
            "SELECT workspace_id FROM memberships \
             WHERE user_id = $1 AND role IN ('editor', 'owner')",
        )
        .bind(user.id)
        .fetch_all(&state.config)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(n,)| n)
        .collect()
    };
    let rows: Vec<(Uuid, String, String, sqlx::types::Json<Value>, Uuid, String)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind, c.config, c.workspace_id, n.name \
         FROM channels c JOIN workspaces n ON n.id = c.workspace_id ORDER BY n.name, c.name",
    )
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind, config, workspace_id, workspace)| {
                let can_edit = user.is_admin || editable.contains(&workspace_id);
                let config = if can_edit {
                    config.0
                } else {
                    crate::notify::redact_secrets(&kind, &config.0)
                };
                GlobalChannelRow {
                    id,
                    name,
                    kind,
                    workspace,
                    workspace_id,
                    can_edit,
                    config,
                }
            })
            .collect(),
    ))
}

/// GET /api/channel-types — the provider manifest the UI renders the form from.
/// No workspace scope: it's static schema, any signed-in user may read it.
pub async fn channel_types(_user: CurrentUser) -> Json<Vec<crate::notify::ProviderMeta>> {
    Json(crate::notify::manifest())
}

#[derive(Serialize)]
pub struct ChannelAlertRow {
    pub id: Uuid,
    /// The rule's target: a monitor/host name, or "All services"/"All hosts" for
    /// a workspace-wide rule.
    pub target: String,
    /// "service" | "host" — what the rule watches, so the UI can group its reach.
    pub kind: String,
    pub workspace: String,
    pub enabled: bool,
    pub firing: Option<bool>,
}

/// GET /api/channels/:id/alerts — the alert rules that notify through this channel
/// (across all workspaces, since channels are shared). Read-only, any signed-in user.
pub async fn channel_alerts(
    State(state): State<AppState>,
    _user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ChannelAlertRow>>, StatusCode> {
    let rows: Vec<(
        Uuid,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
        Option<bool>,
    )> = sqlx::query_as(
        "SELECT r.id, m.name, s.name, r.scope_kind, COALESCE(nt.name, nsc.name), r.enabled, st.firing \
         FROM alert_channels ac JOIN alerts r ON r.id = ac.alert_id \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         LEFT JOIN workspaces nt ON nt.id = COALESCE(m.workspace_id, s.workspace_id) \
         LEFT JOIN workspaces nsc ON nsc.id = r.scope_workspace_id \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE ac.channel_id = $1 ORDER BY r.enabled DESC, nt.name",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, m, s, scope, ws, enabled, firing)| {
                let (target, kind) = match scope.as_deref() {
                    Some("all_services") => ("All services".to_string(), "service"),
                    Some("all_hosts") => ("All hosts".to_string(), "host"),
                    _ if m.is_some() => (m.unwrap(), "service"),
                    _ => (s.unwrap_or_default(), "host"),
                };
                ChannelAlertRow {
                    id,
                    target,
                    kind: kind.to_string(),
                    workspace: ws.unwrap_or_default(),
                    enabled,
                    firing,
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct TestChannelConfig {
    pub kind: String,
    pub config: Value,
}

/// POST /api/workspaces/:id/channels/test — send a test through an unsaved config,
/// so a channel can be verified BEFORE it is created or its edits are saved.
/// Scoped to the workspace (editor+) to avoid an open SSRF relay for any signed-in user.
pub async fn test_channel_config(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
    Json(req): Json<TestChannelConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    rbac::require_role(&state, &user, ws, Role::Editor)
        .await
        .map_err(|s| (s, "forbidden".into()))?;
    if !crate::notify::is_valid_kind(&req.kind) {
        return Err((StatusCode::BAD_REQUEST, "unknown channel kind".into()));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::notify::dispatch(
        &client,
        &req.kind,
        &req.config,
        &crate::notify::Notification::test(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub config: Option<Value>,
}

/// POST /api/workspaces/:id/channels — editors+ add a notification channel.
pub async fn create_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
    Json(req): Json<CreateChannel>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    if !crate::notify::is_valid_kind(&req.kind) || !super::valid_name(&req.name, 64) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO channels (workspace_id, name, kind, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ws)
    .bind(req.name.trim())
    .bind(&req.kind)
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}

#[derive(Deserialize)]
pub struct PatchChannel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// PATCH /api/channels/:id — edit a channel's name / config (editors+).
pub async fn patch_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchChannel>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(
        &state,
        "SELECT workspace_id FROM channels WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    if req
        .name
        .as_deref()
        .is_some_and(|n| !super::valid_name(n, 64))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    sqlx::query(
        "UPDATE channels SET name = COALESCE($2, name), config = COALESCE($3, config) WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.map(|n| n.trim().to_string()))
    .bind(req.config.map(sqlx::types::Json))
    .execute(&state.config)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/channels/:id/test — send a test notification through the channel.
pub async fn test_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ws = ws_of(
        &state,
        "SELECT workspace_id FROM channels WHERE id = $1",
        id,
    )
    .await
    .map_err(|s| (s, "not found".into()))?;
    rbac::require_role(&state, &user, ws, Role::Editor)
        .await
        .map_err(|s| (s, "forbidden".into()))?;
    let row: Option<(String, sqlx::types::Json<Value>)> =
        sqlx::query_as("SELECT kind, config FROM channels WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.config)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (kind, config) = row.ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::notify::dispatch(
        &client,
        &kind,
        &config.0,
        &crate::notify::Notification::test(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/channels/:id
pub async fn delete_channel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(
        &state,
        "SELECT workspace_id FROM channels WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    sqlx::query("DELETE FROM channels WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
