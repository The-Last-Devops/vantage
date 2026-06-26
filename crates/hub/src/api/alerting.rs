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
    let role = rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    // Secrets (tokens/passwords/webhook URLs) go only to those who can edit the
    // channel; viewers get them masked. Editors need the real config to edit.
    let can_see_secrets = role >= Role::Editor;
    let rows: Vec<(Uuid, String, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT id, name, kind, config FROM channels WHERE namespace_id = $1 ORDER BY name",
    )
    .bind(ns)
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
    pub namespace: String,
    pub namespace_id: Uuid,
    /// Whether the caller may edit/delete this channel (editor+ of its namespace).
    pub can_edit: bool,
    pub config: Value,
}

/// GET /api/channels — every channel across all namespaces. Channels are a shared
/// resource: anyone signed in may view them and attach them to alerts. Only an
/// editor of the channel's OWN namespace may edit/delete it, so secrets are shown
/// only to those editors; everyone else gets them masked.
pub async fn list_all_channels(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<GlobalChannelRow>>, StatusCode> {
    // Namespaces the caller can edit (system admins edit everywhere).
    let editable: std::collections::HashSet<Uuid> = if user.is_admin {
        std::collections::HashSet::new()
    } else {
        sqlx::query_as::<_, (Uuid,)>(
            "SELECT namespace_id FROM memberships \
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
        "SELECT c.id, c.name, c.kind, c.config, c.namespace_id, n.name \
         FROM channels c JOIN namespaces n ON n.id = c.namespace_id ORDER BY n.name, c.name",
    )
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, kind, config, namespace_id, namespace)| {
                let can_edit = user.is_admin || editable.contains(&namespace_id);
                let config = if can_edit {
                    config.0
                } else {
                    crate::notify::redact_secrets(&kind, &config.0)
                };
                GlobalChannelRow {
                    id,
                    name,
                    kind,
                    namespace,
                    namespace_id,
                    can_edit,
                    config,
                }
            })
            .collect(),
    ))
}

/// GET /api/channel-types — the provider manifest the UI renders the form from.
/// No namespace scope: it's static schema, any signed-in user may read it.
pub async fn channel_types(_user: CurrentUser) -> Json<Vec<crate::notify::ProviderMeta>> {
    Json(crate::notify::manifest())
}

#[derive(Serialize)]
pub struct ChannelAlertRow {
    pub id: Uuid,
    /// The rule's target: a monitor/host name, or "All services"/"All hosts" for
    /// a namespace-wide rule.
    pub target: String,
    /// "service" | "host" — what the rule watches, so the UI can group its reach.
    pub kind: String,
    pub namespace: String,
    pub enabled: bool,
    pub firing: Option<bool>,
}

/// GET /api/channels/:id/alerts — the alert rules that notify through this channel
/// (across all namespaces, since channels are shared). Read-only, any signed-in user.
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
         LEFT JOIN namespaces nt ON nt.id = COALESCE(m.namespace_id, s.namespace_id) \
         LEFT JOIN namespaces nsc ON nsc.id = r.scope_namespace_id \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE ac.channel_id = $1 ORDER BY r.enabled DESC, nt.name",
    )
    .bind(id)
    .fetch_all(&state.config)
    .await
    .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, m, s, scope, ns, enabled, firing)| {
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
                    namespace: ns.unwrap_or_default(),
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

/// POST /api/namespaces/:id/channels/test — send a test through an unsaved config,
/// so a channel can be verified BEFORE it is created or its edits are saved.
/// Scoped to the namespace (editor+) to avoid an open SSRF relay for any signed-in user.
pub async fn test_channel_config(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<TestChannelConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    rbac::require_role(&state, &user, ns, Role::Editor)
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
    ns: Uuid,
    target_col: &str, // "monitor_id" | "system_id"
    target_id: Uuid,
    scope: &str, // "all_services" | "all_hosts"
) -> Result<Vec<AttachedRule>, StatusCode> {
    let sql = format!(
        "SELECT r.id, r.scope_kind, r.enabled, st.firing FROM alerts r \
         LEFT JOIN alert_state st ON st.alert_id = r.id \
         WHERE r.{target_col} = $1 OR (r.scope_kind = $2 AND r.scope_namespace_id = $3) \
         ORDER BY r.scope_kind NULLS FIRST, r.id"
    );
    let rows: Vec<(Uuid, Option<String>, bool, Option<bool>)> = sqlx::query_as(&sql)
        .bind(target_id)
        .bind(scope)
        .bind(ns)
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

/// GET /api/monitors/:id/alerts — rules covering this service (its own + ns-wide).
pub async fn monitor_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AttachedRule>>, StatusCode> {
    let ns = ns_of(
        &state,
        "SELECT namespace_id FROM monitors WHERE id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    Ok(Json(
        attached_rules(&state, ns, "monitor_id", id, "all_services").await?,
    ))
}

/// GET /api/systems/:id/alerts — rules covering this host (its own + ns-wide).
pub async fn system_alerts(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AttachedRule>>, StatusCode> {
    let ns = ns_of(&state, "SELECT namespace_id FROM systems WHERE id = $1", id).await?;
    rbac::require_role(&state, &user, ns, Role::Viewer).await?;
    Ok(Json(
        attached_rules(&state, ns, "system_id", id, "all_hosts").await?,
    ))
}

#[derive(Serialize)]
pub struct AlertDetail {
    pub id: Uuid,
    pub monitor_id: Option<Uuid>,
    pub system_id: Option<Uuid>,
    pub scope_kind: Option<String>,
    pub scope_namespace_id: Option<Uuid>,
    pub namespace: Option<String>,
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
        "SELECT r.monitor_id, r.system_id, r.scope_kind, r.scope_namespace_id, \
                COALESCE(m.namespace_id, s.namespace_id, r.scope_namespace_id) AS ns_id, \
                n.name AS namespace, m.name AS monitor_name, s.name AS system_name, \
                r.condition, r.renotify_secs \
         FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id \
         LEFT JOIN namespaces n ON n.id = COALESCE(m.namespace_id, s.namespace_id, r.scope_namespace_id) \
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
        ns_id,
        namespace,
        mname,
        sname,
        cond,
        renotify,
    ) = row.ok_or(StatusCode::NOT_FOUND)?;
    rbac::require_role(&state, &user, ns_id, Role::Viewer).await?;

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
        scope_namespace_id: scope_ns,
        namespace,
        target_name,
        condition: cond.0,
        renotify_secs: renotify,
        channels: chans
            .into_iter()
            .map(|(id, name, kind)| ChannelRef { id, name, kind })
            .collect(),
    }))
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
        &crate::notify::Notification::test(),
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
    /// "monitor" | "host" | "all_services" | "all_hosts" + a display name.
    pub target_kind: String,
    pub target_name: String,
    /// Set for namespace-wide rules ("all_services" | "all_hosts").
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
         WHERE COALESCE(m.namespace_id, s.namespace_id, r.scope_namespace_id) = $1 \
         ORDER BY st.firing DESC NULLS LAST, r.enabled DESC, r.id",
    )
    .bind(ns)
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
    /// `monitor_id` or `system_id`, OR a `scope_kind` + `scope_namespace_id`.
    #[serde(default)]
    pub monitor_id: Option<Uuid>,
    #[serde(default)]
    pub system_id: Option<Uuid>,
    #[serde(default)]
    pub scope_kind: Option<String>,
    #[serde(default)]
    pub scope_namespace_id: Option<Uuid>,
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
    // Current namespace (covers scope-wide rules via scope_namespace_id).
    let ns = ns_of(
        &state,
        "SELECT COALESCE(m.namespace_id, s.namespace_id, r.scope_namespace_id) FROM alerts r \
         LEFT JOIN monitors m ON m.id = r.monitor_id \
         LEFT JOIN systems s ON s.id = r.system_id WHERE r.id = $1",
        id,
    )
    .await?;
    rbac::require_role(&state, &user, ns, Role::Editor).await?;

    // Optional re-target. Editor must also be allowed in the NEW target's namespace.
    let retarget = req.monitor_id.is_some() || req.system_id.is_some() || req.scope_kind.is_some();
    if retarget {
        let new_ns = if let Some(mid) = req.monitor_id {
            ns_of(
                &state,
                "SELECT namespace_id FROM monitors WHERE id = $1",
                mid,
            )
            .await?
        } else if let Some(sid) = req.system_id {
            ns_of(
                &state,
                "SELECT namespace_id FROM systems WHERE id = $1",
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
            req.scope_namespace_id.ok_or(StatusCode::BAD_REQUEST)?
        };
        rbac::require_role(&state, &user, new_ns, Role::Editor).await?;
        let scope_ns = if req.scope_kind.is_some() {
            Some(new_ns)
        } else {
            None
        };
        sqlx::query(
            "UPDATE alerts SET monitor_id = $2, system_id = $3, scope_kind = $4, \
                scope_namespace_id = $5 WHERE id = $1",
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
        // regardless of namespace; just require that every id exists.
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
    /// Namespace-wide scope instead of a single target: "all_services" | "all_hosts".
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

/// POST /api/namespaces/:id/alerts — editors+ create an alert rule.
pub async fn create_alert(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(ns): Path<Uuid>,
    Json(req): Json<CreateAlert>,
) -> Result<Json<Uuid>, StatusCode> {
    rbac::require_role(&state, &user, ns, Role::Editor).await?;
    // Either a specific target (monitor/system) OR a namespace-wide scope.
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
    // regardless of namespace; just require that every id exists.
    let valid: Option<(i64,)> =
        sqlx::query_as("SELECT count(DISTINCT id) FROM channels WHERE id = ANY($1)")
            .bind(&req.channel_ids)
            .fetch_optional(&state.config)
            .await
            .map_err(internal)?;
    if valid.map(|(n,)| n as usize) != Some(req.channel_ids.len()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // For a ns-wide rule the target columns are null and the scope columns are set
    // (scope namespace = the path's namespace).
    let (mon, sys, scope_ns) = match scope_kind {
        Some(_) => (None, None, Some(ns)),
        None => (req.monitor_id, req.system_id, None),
    };
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO alerts (monitor_id, system_id, scope_kind, scope_namespace_id, \
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
