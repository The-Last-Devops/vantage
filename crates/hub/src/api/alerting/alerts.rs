//! Alert-rule API: list/get/create/patch/delete/test rules, recent alert
//! events, and the "covering rules" views for a monitor or host.

use super::*;

#[derive(Serialize)]
pub struct AttachedRule {
    pub id: Uuid,
    /// null = this rule targets the object specifically; else "all_services"/"all_hosts".
    pub scope_kind: Option<String>,
    pub enabled: bool,
    pub firing: Option<bool>,
}

async fn attached_rules(
    state: &AppState,
    ws: Uuid,
    target_col: &str, // "monitor_id" | "system_id"
    target_id: Uuid,
    scope: &str, // "all_services" | "all_hosts"
) -> Result<Vec<AttachedRule>, StatusCode> {
    let sql = format!(
        "SELECT r.id, r.scope_kind, r.enabled, st.firing FROM alerts r \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE r.{target_col} = $1 OR (r.scope_kind = $2 AND r.scope_workspace_id = $3) \
         ORDER BY r.scope_kind NULLS FIRST, r.id"
    );
    let rows: Vec<(Uuid, Option<String>, bool, Option<bool>)> = sqlx::query_as(&sql)
        .bind(target_id)
        .bind(scope)
        .bind(ws)
        .fetch_all(&state.config)
        .await
        .map_err(internal)?;
    Ok(rows
        .into_iter()
        .map(|(id, scope_kind, enabled, firing)| AttachedRule {
            id,
            scope_kind,
            enabled,
            firing,
        })
        .collect())
}

/// GET /api/monitors/:id/alerts — rules covering this service (its own + ws-wide).
pub async fn monitor_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AttachedRule>>, StatusCode> {
    let ws = ws_of(
        &state,
        "SELECT workspace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    Ok(Json(
        attached_rules(&state, ws, "monitor_id", id, "all_services").await?,
    ))
}

/// GET /api/systems/:id/alerts — rules covering this host (its own + ws-wide).
pub async fn system_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AttachedRule>>, StatusCode> {
    let ws = ws_of(&state, "SELECT workspace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    Ok(Json(
        attached_rules(&state, ws, "system_id", id, "all_hosts").await?,
    ))
}

#[derive(Serialize)]
pub struct AlertDetail {
    pub id: Uuid,
    pub monitor_id: Option<Uuid>,
    pub system_id: Option<Uuid>,
    pub scope_kind: Option<String>,
    pub scope_workspace_id: Option<Uuid>,
    pub workspace: Option<String>,
    pub target_name: String,
    pub condition: Value,
    pub renotify_secs: Option<i32>,
    pub channels: Vec<ChannelRef>,
}

/// GET /api/alerts/:id — one rule with everything the editor needs to populate.
pub async fn get_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<AlertDetail>, StatusCode> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Option<Uuid>,
        Option<Uuid>,
        Option<String>,
        Option<Uuid>,
        Uuid,
        Option<String>,
        Option<String>,
        Option<String>,
        sqlx::types::Json<Value>,
        Option<i32>,
    )> = sqlx::query_as(
        "SELECT r.monitor_id, r.system_id, r.scope_kind, r.scope_workspace_id, \
                COALESCE(m.workspace_id, s.workspace_id, r.scope_workspace_id) AS ws_id, \
                n.name AS workspace, m.name AS monitor_name, s.name AS system_name, \
                r.condition, r.renotify_secs \
         FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         LEFT JOIN workspaces n ON n.id = COALESCE(m.workspace_id, s.workspace_id, r.scope_workspace_id) \
         WHERE r.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.config)
    .await
    .map_err(internal)?;
    let (
        monitor_id,
        system_id,
        scope_kind,
        scope_ns,
        ws_id,
        workspace,
        mname,
        sname,
        cond,
        renotify,
    ) = row.ok_or(StatusCode::NOT_FOUND)?;
    rbac::require_role(&state, &user, ws_id, Role::Viewer).await?;

    let chans: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT c.id, c.name, c.kind FROM alert_channels ac \
         JOIN channels c ON c.id = ac.channel_id WHERE ac.alert_id = $1 ORDER BY c.name",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    let target_name = match scope_kind.as_deref() {
        Some("all_services") => "All services".into(),
        Some("all_hosts") => "All hosts".into(),
        _ => mname.or(sname).unwrap_or_default(),
    };
    Ok(Json(AlertDetail {
        id,
        monitor_id,
        system_id,
        scope_kind,
        scope_workspace_id: scope_ns,
        workspace,
        target_name,
        condition: cond.0,
        renotify_secs: renotify,
        channels: chans
            .into_iter()
            .map(|(id, name, kind)| ChannelRef { id, name, kind })
            .collect(),
    }))
}

// ---- alert rules ------------------------------------------------------------

#[derive(Serialize)]
pub struct ChannelRef {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
}

#[derive(Serialize)]
pub struct AlertRow {
    pub id: Uuid,
    pub monitor_id: Option<Uuid>,
    pub system_id: Option<Uuid>,
    /// Every channel this rule fans out to.
    pub channels: Vec<ChannelRef>,
    /// "monitor" | "host" | "all_services" | "all_hosts" + a display name.
    pub target_kind: String,
    pub target_name: String,
    /// Set for workspace-wide rules ("all_services" | "all_hosts").
    pub scope_kind: Option<String>,
    pub cooldown_secs: i32,
    /// Re-notify cadence while firing; null = notify once.
    pub renotify_secs: Option<i32>,
    pub enabled: bool,
    pub condition: Value,
    /// Current state from the engine (null = not evaluated yet).
    pub firing: Option<bool>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(sqlx::FromRow)]
struct AlertJoin {
    id: Uuid,
    monitor_id: Option<Uuid>,
    system_id: Option<Uuid>,
    scope_kind: Option<String>,
    cooldown_secs: i32,
    renotify_secs: Option<i32>,
    enabled: bool,
    condition: sqlx::types::Json<Value>,
    monitor_name: Option<String>,
    system_name: Option<String>,
    firing: Option<bool>,
    since: Option<chrono::DateTime<chrono::Utc>>,
    channel_id: Option<Uuid>,
    channel_name: Option<String>,
    channel_kind: Option<String>,
}

/// GET /api/workspaces/:id/alerts — rules whose target lives in this workspace,
/// with their channels, target name and current firing state. One row per
/// rule×channel is collapsed into a rule with a channels[] list.
pub async fn list_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
) -> Result<Json<Vec<AlertRow>>, StatusCode> {
    rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    let rows: Vec<AlertJoin> = sqlx::query_as(
        "SELECT r.id, r.monitor_id, r.system_id, r.scope_kind, r.cooldown_secs, r.renotify_secs, r.enabled, \
                r.condition, m.name AS monitor_name, s.name AS system_name, \
                st.firing, st.last_changed AS since, \
                c.id AS channel_id, c.name AS channel_name, c.kind AS channel_kind \
         FROM alerts r \
         LEFT JOIN alert_channels ac ON ac.alert_id = r.id \
         LEFT JOIN channels c ON c.id = ac.channel_id \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE COALESCE(m.workspace_id, s.workspace_id, r.scope_workspace_id) = $1 \
         ORDER BY st.firing DESC NULLS LAST, r.enabled DESC, r.id",
    )
    .bind(ws)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    let mut out: Vec<AlertRow> = Vec::new();
    for a in rows {
        if out.last().map(|r| r.id) != Some(a.id) {
            let (target_kind, target_name) = match a.scope_kind.as_deref() {
                Some("all_services") => ("all_services", "All services".to_string()),
                Some("all_hosts") => ("all_hosts", "All hosts".to_string()),
                _ if a.monitor_id.is_some() => {
                    ("monitor", a.monitor_name.clone().unwrap_or_default())
                }
                _ => ("host", a.system_name.clone().unwrap_or_default()),
            };
            out.push(AlertRow {
                id: a.id,
                monitor_id: a.monitor_id,
                system_id: a.system_id,
                channels: Vec::new(),
                target_kind: target_kind.into(),
                target_name,
                scope_kind: a.scope_kind.clone(),
                cooldown_secs: a.cooldown_secs,
                renotify_secs: a.renotify_secs,
                enabled: a.enabled,
                condition: a.condition.0,
                firing: a.firing,
                since: a.since,
            });
        }
        if let (Some(id), Some(name), Some(kind)) = (a.channel_id, a.channel_name, a.channel_kind) {
            out.last_mut()
                .unwrap()
                .channels
                .push(ChannelRef { id, name, kind });
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct PatchAlert {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub cooldown_secs: Option<i32>,
    /// Full replacement of the rule's channels. Present only from the editor.
    #[serde(default)]
    pub channel_ids: Option<Vec<Uuid>>,
    /// Re-notify cadence; null = off. Applied together with channel_ids.
    #[serde(default)]
    pub renotify_secs: Option<i32>,
    #[serde(default)]
    pub condition: Option<Value>,
    /// Optional re-target (the editor's "source" change). Provide a specific
    /// `monitor_id` or `system_id`, OR a `scope_kind` + `scope_workspace_id`.
    #[serde(default)]
    pub monitor_id: Option<Uuid>,
    #[serde(default)]
    pub system_id: Option<Uuid>,
    #[serde(default)]
    pub scope_kind: Option<String>,
    #[serde(default)]
    pub scope_workspace_id: Option<Uuid>,
}

/// PATCH /api/alerts/:id — toggle enabled, edit condition/channels/cadence, and
/// optionally re-target the rule's source. A bare `{enabled}` toggle leaves the
/// rest untouched; the editor sends `channel_ids` (also commits `renotify_secs`).
pub async fn patch_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchAlert>,
) -> Result<StatusCode, StatusCode> {
    // Current workspace (covers scope-wide rules via scope_workspace_id).
    let ws = ws_of(
        &state,
        "SELECT COALESCE(m.workspace_id, s.workspace_id, r.scope_workspace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;

    // Optional re-target. Editor must also be allowed in the NEW target's workspace.
    let retarget = req.monitor_id.is_some() || req.system_id.is_some() || req.scope_kind.is_some();
    if retarget {
        let new_ns = if let Some(mid) = req.monitor_id {
            ws_of(
                &state,
                "SELECT workspace_id FROM monitors WHERE id = $1",
                mid,
            )
            .await?
        } else if let Some(sid) = req.system_id {
            ws_of(
                &state,
                "SELECT workspace_id FROM systems WHERE id = $1",
                sid,
            )
            .await?
        } else {
            // scope rule
            if !matches!(
                req.scope_kind.as_deref(),
                Some("all_services") | Some("all_hosts")
            ) {
                return Err(StatusCode::BAD_REQUEST);
            }
            req.scope_workspace_id.ok_or(StatusCode::BAD_REQUEST)?
        };
        rbac::require_role(&state, &user, new_ns, Role::Editor).await?;
        let scope_ns = if req.scope_kind.is_some() {
            Some(new_ns)
        } else {
            None
        };
        sqlx::query(
            "UPDATE alerts SET monitor_id = $2, system_id = $3, scope_kind = $4, \
                scope_workspace_id = $5 WHERE id = $1",
        )
        .bind(id)
        .bind(req.monitor_id)
        .bind(req.system_id)
        .bind(&req.scope_kind)
        .bind(scope_ns)
        .execute(&state.config)
        .await
        .map_err(internal)?;
        // Re-pointing the rule invalidates its firing state — clear it.
        let _ = sqlx::query("DELETE FROM alert_state WHERE alert_id = $1")
            .bind(id)
            .execute(&state.config)
            .await;
    }

    sqlx::query(
        "UPDATE alerts SET enabled = COALESCE($2, enabled), \
            cooldown_secs = COALESCE($3, cooldown_secs), \
            condition = COALESCE($4, condition) WHERE id = $1",
    )
    .bind(id)
    .bind(req.enabled)
    .bind(req.cooldown_secs)
    .bind(req.condition.map(sqlx::types::Json))
    .execute(&state.config)
    .await
    .map_err(internal)?;

    // Channel set + renotify are replaced together (editor save only).
    if let Some(ids) = req.channel_ids {
        // Channels are a shared resource — a rule may use any existing channel,
        // regardless of workspace; just require that every id exists.
        let valid: Option<(i64,)> =
            sqlx::query_as("SELECT count(DISTINCT id) FROM channels WHERE id = ANY($1)")
                .bind(&ids)
                .fetch_optional(&state.config)
                .await
                .map_err(internal)?;
        if valid.map(|(n,)| n as usize) != Some(ids.len()) {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query("UPDATE alerts SET renotify_secs = $2 WHERE id = $1")
            .bind(id)
            .bind(req.renotify_secs)
            .execute(&state.config)
            .await
            .map_err(internal)?;
        sqlx::query("DELETE FROM alert_channels WHERE alert_id = $1")
            .bind(id)
            .execute(&state.config)
            .await
            .map_err(internal)?;
        for cid in &ids {
            sqlx::query("INSERT INTO alert_channels (alert_id, channel_id) VALUES ($1, $2)")
                .bind(id)
                .bind(cid)
                .execute(&state.config)
                .await
                .map_err(internal)?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/alerts/:id/test — send a test notification through the rule's channel.
pub async fn test_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ws = ws_of(
        &state,
        "SELECT COALESCE(m.workspace_id, s.workspace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await
    .map_err(|s| (s, "not found".into()))?;
    rbac::require_role(&state, &user, ws, Role::Editor)
        .await
        .map_err(|s| (s, "forbidden".into()))?;
    let channels: Vec<(String, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT c.name, c.kind, c.config FROM alert_channels ac \
         JOIN channels c ON c.id = ac.channel_id WHERE ac.alert_id = $1",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if channels.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "rule has no channels".into()));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Send to every channel; report which ones failed.
    let test = crate::notify::Notification::test();
    let mut errors = Vec::new();
    for (name, kind, config) in &channels {
        if let Err(e) = crate::notify::dispatch(&client, kind, &config.0, &test).await {
            errors.push(format!("{name}: {e}"));
        }
    }
    if errors.is_empty() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::BAD_GATEWAY, errors.join("; ")))
    }
}

#[derive(Serialize)]
pub struct AlertEvent {
    pub alert_id: Uuid,
    pub at: chrono::DateTime<chrono::Utc>,
    pub firing: bool,
    pub message: Option<String>,
    pub target_name: String,
    pub target_kind: String,
}

/// GET /api/workspaces/:id/alert-events — recent fired/recovered transitions.
pub async fn alert_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
) -> Result<Json<Vec<AlertEvent>>, StatusCode> {
    rbac::require_role(&state, &user, ws, Role::Viewer).await?;
    let rows: Vec<(
        Uuid,
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT e.alert_id, e.at, e.firing, e.message, m.name, s.name \
         FROM alert_events e JOIN alerts r ON r.id = e.alert_id \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         WHERE COALESCE(m.workspace_id, s.workspace_id) = $1 \
         ORDER BY e.at DESC LIMIT 100",
    )
    .bind(ws)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(alert_id, at, firing, message, mname, sname)| {
                let (target_kind, target_name) = match mname {
                    Some(n) => ("monitor", n),
                    None => ("host", sname.unwrap_or_default()),
                };
                AlertEvent {
                    alert_id,
                    at,
                    firing,
                    message,
                    target_name,
                    target_kind: target_kind.into(),
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct CreateAlert {
    #[serde(default)]
    pub monitor_id: Option<Uuid>,
    #[serde(default)]
    pub system_id: Option<Uuid>,
    /// Workspace-wide scope instead of a single target: "all_services" | "all_hosts".
    #[serde(default)]
    pub scope_kind: Option<String>,
    pub channel_ids: Vec<Uuid>,
    #[serde(default)]
    pub condition: Option<Value>,
    #[serde(default)]
    pub cooldown_secs: Option<i32>,
    #[serde(default)]
    pub renotify_secs: Option<i32>,
}

/// POST /api/workspaces/:id/alerts — editors+ create an alert rule.
pub async fn create_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ws): Path<Uuid>,
    Json(req): Json<CreateAlert>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    // Either a specific target (monitor/system) OR a workspace-wide scope.
    let scope_kind = req.scope_kind.as_deref().filter(|s| !s.is_empty());
    if let Some(k) = scope_kind {
        if !matches!(k, "all_services" | "all_hosts") {
            return Err(StatusCode::BAD_REQUEST);
        }
    } else if req.monitor_id.is_none() && req.system_id.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.channel_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Channels are a shared resource — a rule may use any existing channel,
    // regardless of workspace; just require that every id exists.
    let valid: Option<(i64,)> =
        sqlx::query_as("SELECT count(DISTINCT id) FROM channels WHERE id = ANY($1)")
            .bind(&req.channel_ids)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;
    if valid.map(|(n,)| n as usize) != Some(req.channel_ids.len()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // For a ws-wide rule the target columns are null and the scope columns are set
    // (scope workspace = the path's workspace).
    let (mon, sys, scope_ns) = match scope_kind {
        Some(_) => (None, None, Some(ws)),
        None => (req.monitor_id, req.system_id, None),
    };
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO alerts (monitor_id, system_id, scope_kind, scope_workspace_id, \
            condition, cooldown_secs, renotify_secs) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(mon)
    .bind(sys)
    .bind(scope_kind)
    .bind(scope_ns)
    .bind(sqlx::types::Json(
        req.condition.unwrap_or_else(|| serde_json::json!({})),
    ))
    .bind(req.cooldown_secs.unwrap_or(300).max(0))
    .bind(req.renotify_secs)
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    for cid in &req.channel_ids {
        sqlx::query("INSERT INTO alert_channels (alert_id, channel_id) VALUES ($1, $2)")
            .bind(id)
            .bind(cid)
            .execute(&state.config)
            .await
            .map_err(internal)?;
    }
    Ok(Json(id))
}

/// DELETE /api/alerts/:id (workspace resolved via the rule's monitor/server).
pub async fn delete_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let ws = ws_of(
        &state,
        "SELECT COALESCE(m.workspace_id, s.workspace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ws, Role::Editor).await?;
    sqlx::query("DELETE FROM alerts WHERE id = $1")
        .bind(id)
        .execute(&state.config)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
