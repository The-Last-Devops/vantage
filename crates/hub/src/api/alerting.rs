use super::*;

#[derive(Serialize)]
pub struct ChannelRow {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub config: Value,
}

/// GET /api/namespaces/:id/channels
pub async fn list_channels(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<ChannelRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<(Uuid, String, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT id, name, kind, config FROM channels WHERE namespace_id = $1 ORDER BY name",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind, config)| ChannelRow {
                id,
                name,
                kind,
                config: config.0,
            })
            .collect(),
    ))
}

/// GET /api/channel-types — the provider manifest the UI renders the form from.
/// No namespace scope: it's static schema, any signed-in user may read it.
pub async fn channel_types(_user: CurrentUser) -> Json<Vec<crate::notify::ProviderMeta>> {
    Json(crate::notify::manifest())
}

#[derive(Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub kind: String,
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
    if !crate::notify::is_valid_kind(&req.kind) || !super::valid_name(&req.name, 64) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO channels (namespace_id, name, kind, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ns)
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
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM channels WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
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
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM channels WHERE id = $1",
        id,
    )
    .await
    .map_err(|s| (s, "not found".into()))?;
    rbac::require_role(&state, &user, ns, Role::Editor)
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
        "✅ Test notification from Last Monitor — this channel works.",
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
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
    /// "monitor" | "host" + the target's display name.
    pub target_kind: String,
    pub target_name: String,
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

/// GET /api/namespaces/:id/alerts — rules whose target lives in this namespace,
/// with their channels, target name and current firing state. One row per
/// rule×channel is collapsed into a rule with a channels[] list.
pub async fn list_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<AlertRow>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    let rows: Vec<AlertJoin> = sqlx::query_as(
        "SELECT r.id, r.monitor_id, r.system_id, r.cooldown_secs, r.renotify_secs, r.enabled, \
                r.condition, m.name AS monitor_name, s.name AS system_name, \
                st.firing, st.last_changed AS since, \
                c.id AS channel_id, c.name AS channel_name, c.kind AS channel_kind \
         FROM alerts r \
         LEFT JOIN alert_channels ac ON ac.alert_id = r.id \
         LEFT JOIN channels c ON c.id = ac.channel_id \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE COALESCE(m.namespace_id, s.namespace_id) = $1 \
         ORDER BY st.firing DESC NULLS LAST, r.enabled DESC, r.id",
    )
    .bind(ns)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;

    let mut out: Vec<AlertRow> = Vec::new();
    for a in rows {
        if out.last().map(|r| r.id) != Some(a.id) {
            let (target_kind, target_name) = if a.monitor_id.is_some() {
                ("monitor", a.monitor_name.clone().unwrap_or_default())
            } else {
                ("host", a.system_name.clone().unwrap_or_default())
            };
            out.push(AlertRow {
                id: a.id,
                monitor_id: a.monitor_id,
                system_id: a.system_id,
                channels: Vec::new(),
                target_kind: target_kind.into(),
                target_name,
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
}

/// PATCH /api/alerts/:id — toggle enabled, edit condition/channels/cadence.
/// A bare `{enabled}` toggle leaves channels and renotify untouched; the editor
/// sends `channel_ids` (which also commits `renotify_secs`, allowing null = off).
pub async fn patch_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchAlert>,
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
        // Every channel must belong to this namespace.
        let valid: Option<(i64,)> = sqlx::query_as(
            "SELECT count(*) FROM channels WHERE id = ANY($1) AND namespace_id = $2",
        )
        .bind(&ids)
        .bind(ns)
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
    let ns = ns_of(
        &state,
        "SELECT COALESCE(m.namespace_id, s.namespace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await
    .map_err(|s| (s, "not found".into()))?;
    rbac::require_role(&state, &user, ns, Role::Editor)
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
    let mut errors = Vec::new();
    for (name, kind, config) in &channels {
        if let Err(e) = crate::notify::dispatch(
            &client,
            kind,
            &config.0,
            "🔔 Test alert from Last Monitor — this rule's channels work.",
        )
        .await
        {
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

/// GET /api/namespaces/:id/alert-events — recent fired/recovered transitions.
pub async fn alert_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
) -> Result<Json<Vec<AlertEvent>>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
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
         WHERE COALESCE(m.namespace_id, s.namespace_id) = $1 \
         ORDER BY e.at DESC LIMIT 100",
    )
    .bind(ns)
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
    pub channel_ids: Vec<Uuid>,
    #[serde(default)]
    pub condition: Option<Value>,
    #[serde(default)]
    pub cooldown_secs: Option<i32>,
    #[serde(default)]
    pub renotify_secs: Option<i32>,
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
    if req.channel_ids.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Every channel must belong to this namespace.
    let valid: Option<(i64,)> =
        sqlx::query_as("SELECT count(*) FROM channels WHERE id = ANY($1) AND namespace_id = $2")
            .bind(&req.channel_ids)
            .bind(ns)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;
    if valid.map(|(n,)| n as usize) != Some(req.channel_ids.len()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO alerts (monitor_id, system_id, condition, cooldown_secs, renotify_secs) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(req.monitor_id)
    .bind(req.system_id)
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

// ---- edit / delete ----------------------------------------------------------

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
    // slug is part of the public URL → strict identifier; title is a display name.
    if !super::valid_ns_name(&req.slug) || !super::valid_name(&req.title, 120) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO status_pages (namespace_id, slug, title, config) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ns)
    .bind(&req.slug)
    .bind(req.title.trim())
    .bind(sqlx::types::Json(
        req.config.unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(id))
}
